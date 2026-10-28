//! Window and tabpage layout, management, and navigation (`src/nvim/window.c`).

use crate::api::private::helpers::{
    TryState, api_clear_error, api_set_error, cstr_as_string, dict_obj, error_set,
    find_window_by_handle, integer_obj, try_enter, try_leave,
};
use crate::api::private::validate::{Dict, Error, ErrorType, KeyValuePair};
use crate::api::win_config::{
    FR_COL, K_CTX_NO_DISPLAY, K_CTX_NO_EVENTS, K_FLOAT_ANCHOR_EAST, K_FLOAT_ANCHOR_SOUTH,
    K_FLOAT_RELATIVE_LASTSTATUS, K_FLOAT_RELATIVE_TABLINE, K_FLOAT_RELATIVE_WINDOW, K_UIMULTIGRID,
    K_WIN_STYLE_MINIMAL, WIN_CONFIG_INIT,
};
use crate::autocmd::{
    AUGROUP_ALL, EVENT_BUFENTER, EVENT_BUFLEAVE, EVENT_TABCLOSED, EVENT_TABCLOSEDPRE,
    EVENT_TABENTER, EVENT_TABLEAVE, EVENT_TABMOVED, EVENT_TABNEW, EVENT_TABNEWENTERED,
    EVENT_WINCLOSED, EVENT_WINENTER, EVENT_WINLEAVE, EVENT_WINNEW, EVENT_WINNEWPRE,
    EVENT_WINRESIZED, EVENT_WINSCROLLED, apply_autocmds, apply_autocmds_win, aucmd_defer,
    block_autocmds, event_ignored, has_event, unblock_autocmds,
};
use crate::buffer::{
    BLN_LISTED, Bufref, DOBUF_FIRST, DOBUF_GOTO, DOBUF_UNLOAD, FileBuf, GETF_ALT, K_UIMESSAGES,
    MAXLNUM, NOTDONE, VALID_TOPLINE, VAR_SCOPE, bt_help, bt_prompt, bt_quickfix, buf_hide,
    buf_valid, buflist_findname_exp, buflist_findnr, buflist_getfile, buflist_new, bufref_valid,
    close_buffer, do_autochdir, do_buffer, maketitle, set_bufref,
};
use crate::charset::{ascii_isdigit, getdigits_int};
use crate::cmdexpand::{CTRL_N, CTRL_P, Cmdidx};
use crate::context::{CdCause, CtxSwitch, K_CD_CAUSE_WINDOW, ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::{Colnr, CursorPos, Linenr, check_cursor, check_cursor_lnum, equalpos};
use crate::decoration::decor_conceal_line;
use crate::diff::{diff_clear, diffopt_closeoff};
use crate::drawscreen::{
    FR_LEAF, FR_ROW, UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, comp_col, redraw_all_later,
    redraw_later, redraw_winline, showmode, status_redraw_all,
};
use crate::errors::*;
use crate::eval::typval::{
    TvDict, TvList, TypVal, TypvalVval, VarType, tv_dict_add_dict, tv_dict_add_list,
    tv_dict_add_tv, tv_dict_alloc, tv_dict_extend, tv_dict_set_keys_readonly, tv_dict_unref,
    tv_list_alloc, tv_list_append_owned_tv,
};
use crate::eval::vars::{init_var_dict, unref_var_dict, vars_clear};
use crate::eval::window::win_has_winnr;
use crate::eval::{get_v_event, restore_v_event};
use crate::ex_cmds::{BL_FIX, BL_SOL, CTRL_C_CONST, ECMD_HIDE, ECMD_LASTL, do_ecmd};
use crate::ex_cmds2::{can_abandon, dialog_changed};
use crate::ex_docmd::{
    ACTION_SPLIT, CdScope, FIND_ANY, FIND_DEFINE, do_cmdline_cmd, tabpage_close,
};
use crate::ex_eval::aborting;
use crate::ex_getln::{
    CTRL_B_CONST, CTRL_CONST_CONST, CTRL_D_CONST, CTRL_F_CONST, CTRL_G, CTRL_HAT_CONST,
    CTRL_K_CONST, CTRL_L_CONST, CTRL_O, CTRL_Q_CONST, CTRL_R_CONST, CTRL_RSB_CONST, CTRL_T_CONST,
    CTRL_W_CONST, CTRL_Z_CONST, compute_cmdrow, curbuf_locked, is_in_cmdwin, text_locked,
    text_locked_msg, text_or_buf_locked,
};
use crate::file_search::{do_autocmd_dirchanged, grab_file_name};
use crate::fileio::shorten_fnames;
use crate::fold::{
    FoldEntry, clear_folding, copy_folding_state, fold_init_win, get_deepest_nesting, has_folding,
};
use crate::globals::*;
use crate::grid::{
    GridView, ScreenGrid, grid_adjust, grid_assign_handle, grid_clear, grid_free, win_grid_alloc,
};
use crate::input::{beep_flush, langmap_adjust, plain_vgetc};
use crate::insert::{
    CTRL_J_CONST, CTRL_S_CONST, CTRL_X_CONST, beginline, cursor_down_inner, cursor_up_inner,
};
use crate::keycodes::{CAR, K_BS, K_DOWN, K_KENTER, K_LEFT, K_RIGHT, K_UP, TAB};
use crate::main_init::getout;
use crate::mark::{copy_jumplist, free_jumplist, setmark, setpcmark};
use crate::r#match::clear_matches;
use crate::math::{FAIL, OK};
use crate::memory::xstrlcpy;
use crate::message::{
    K_ZINDEX_MESSAGES, emsg, iemsg, internal_error, msg, msg_clr_eos_force, msg_grid_validate,
    msg_ui_flush, semsg, set_keep_msg,
};
use crate::mouse::{reset_dragwin, setmouse};
use crate::r#move::{
    changed_line_abv_curs, changed_line_abv_curs_win, curs_columns, invalidate_botline_win,
    set_topline, textpos2screenpos, update_topline, validate_botline_win, validate_cursor,
    win_col_off, win_col_off2,
};
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use crate::normal::{
    FIND_IDENT, add_to_showcmd, check_text_or_curbuf_locked, do_nv_ident, find_ident_under_cursor,
    reset_visual_and_resel,
};
use crate::option::{
    K_OPT_CMDHEIGHT, K_OPT_WINDOW, OptInt, buf_copy_options, clear_winopt, get_scrolloff_value,
    option_was_set, set_option_value, win_copy_options,
};
use crate::option_vars::*;
use crate::optionstr::Optset;
use crate::os::fs::{os_chdir, os_dirname};
use crate::path::{K_PATH_CMP_LITERAL, path_equal};
use crate::plines::{plines_win, plines_win_col, plines_win_nofill, win_text_height};
use crate::popupmenu::pum_pos_ui_flush;
use crate::quickfix::{K_OPT_SWB_FLAG_USETAB, copy_loclist_stack, qf_free_all, qf_view_result};
use crate::search::find_pattern_in_path;
use crate::state::{
    MODE_CMDLINE, MODE_INSERT, MODE_NORMAL, MODE_TERMINAL, SaveVEvent, get_real_state,
    virtual_active,
};
use crate::statusline::{K_UITABLINE, stl_clear_click_defs};
use crate::strings::{gettext, vim_snprintf};
use crate::syntax::reset_synblock;
use crate::tag::{K_OPT_SWB_FLAG_USEOPEN, tagstack_clear_entry};
use crate::terminal::terminal_check_size;
use crate::ui::{
    ui_call_grid_destroy, ui_call_win_close, ui_call_win_external_pos, ui_call_win_float_pos,
    ui_call_win_hide, ui_call_win_pos, ui_call_win_viewport, ui_call_win_viewport_margins,
    ui_check_cursor_grid, ui_has,
};
use crate::ui_compositor::{ui_comp_layers_adjust, ui_comp_put_grid, ui_comp_remove_grid};
use crate::undo::{buf_is_changed_fn, u_sync};
use crate::winfloat::{
    win_border_height, win_border_width, win_config_float, win_float_anchor_laststatus,
    win_float_find_altwin, win_float_update_statusline, win_new_float, win_reconfig_floats,
};
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

pub type WinKind = c_int;
pub(crate) const VALID_BOTLINE_AP: c_int = 0x40;

const K_WIN_OPT_COUNT: usize = 53;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PosSave {
    pub w_topline_save: c_int,
    pub w_topline_corr: c_int,
    pub w_cursor_save: CursorPos,
    pub w_cursor_corr: CursorPos,
}

#[repr(C)]
#[derive(Clone, Default)]
pub struct LcsChars {
    pub eol: u32,
    pub ext: u32,
    pub prec: u32,
    pub nbsp: u32,
    pub space: u32,
    pub tab1: u32,
    pub tab2: u32,
    pub tab3: u32,
    pub leadtab1: u32,
    pub leadtab2: u32,
    pub leadtab3: u32,
    pub lead: u32,
    pub trail: u32,
    pub multispace: Vec<u32>,
    pub leadmultispace: Vec<u32>,
    pub conceal: u32,
}

impl LcsChars {
    pub const fn new() -> Self {
        Self {
            eol: 0,
            ext: 0,
            prec: 0,
            nbsp: 0,
            space: 0,
            tab1: 0,
            tab2: 0,
            tab3: 0,
            leadtab1: 0,
            leadtab2: 0,
            leadtab3: 0,
            lead: 0,
            trail: 0,
            multispace: Vec::new(),
            leadmultispace: Vec::new(),
            conceal: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FcsChars {
    pub stl: u32,
    pub stlnc: u32,
    pub wbr: u32,
    pub horiz: u32,
    pub horizup: u32,
    pub horizdown: u32,
    pub vert: u32,
    pub vertleft: u32,
    pub vertright: u32,
    pub verthoriz: u32,
    pub fold: u32,
    pub foldopen: u32,
    pub foldclosed: u32,
    pub foldsep: u32,
    pub foldinner: u32,
    pub diff: u32,
    pub msgsep: u32,
    pub eob: u32,
    pub lastline: u32,
    pub trunc: u32,
    pub truncrl: u32,
}

#[repr(C)]
pub struct Winopt {
    pub wo_arab: c_int,
    pub wo_bri: c_int,
    pub wo_briopt: *mut c_char,
    pub wo_diff: c_int,
    pub wo_fdc: *mut c_char,
    pub wo_eiw: *mut c_char,
    pub wo_fdc_save: *mut c_char,
    pub wo_fen: c_int,
    pub wo_fen_save: c_int,
    pub wo_fdi: *mut c_char,
    pub wo_fdl: i64,
    pub wo_fdl_save: i64,
    pub wo_fdm: *mut c_char,
    pub wo_fdm_save: *mut c_char,
    pub wo_fml: i64,
    pub wo_fdn: i64,
    pub wo_fde: crate::eval::typval::Callback,
    pub wo_fdt: crate::eval::typval::Callback,
    pub wo_fmr: *mut c_char,
    pub wo_lbr: c_int,
    pub wo_list: c_int,
    pub wo_nu: c_int,
    pub wo_rnu: c_int,
    pub wo_ve: *mut c_char,
    pub wo_ve_flags: c_uint,
    pub wo_nuw: i64,
    pub wo_wfb: c_int,
    pub wo_wfh: c_int,
    pub wo_wfw: c_int,
    pub wo_wp: c_int,
    pub wo_pvw: c_int,
    pub wo_lhi: i64,
    pub wo_rl: c_int,
    pub wo_rlc: *mut c_char,
    pub wo_scr: i64,
    pub wo_sms: c_int,
    pub wo_spell: c_int,
    pub wo_cuc: c_int,
    pub wo_cul: c_int,
    pub wo_culopt: *mut c_char,
    pub wo_cc: *mut c_char,
    pub wo_sbr: *mut c_char,
    pub wo_stc: *mut c_char,
    pub wo_stl: *mut c_char,
    pub wo_wbr: *mut c_char,
    pub wo_scb: c_int,
    pub wo_diff_saved: c_int,
    pub wo_scb_save: c_int,
    pub wo_wrap: c_int,
    pub wo_wrap_save: c_int,
    pub wo_cocu: *mut c_char,
    pub wo_cole: i64,
    pub wo_crb: c_int,
    pub wo_crb_save: c_int,
    pub wo_scl: *mut c_char,
    pub wo_siso: i64,
    pub wo_so: i64,
    pub wo_sop: i64,
    pub wo_winhl: *mut c_char,
    pub wo_lcs: *mut c_char,
    pub wo_fcs: *mut c_char,
    pub wo_winbl: i64,
    pub wo_wrap_flags: u32,
    pub wo_stl_flags: u32,
    pub wo_wbr_flags: u32,
    pub wo_fde_flags: u32,
    pub wo_fdt_flags: u32,
    pub wo_script_ctx: [crate::runtime::Sctx; K_WIN_OPT_COUNT],
}

impl Winopt {
    pub const ZERO: Self = Self {
        wo_arab: 0,
        wo_bri: 0,
        wo_briopt: ptr::null_mut(),
        wo_diff: 0,
        wo_fdc: ptr::null_mut(),
        wo_eiw: ptr::null_mut(),
        wo_fdc_save: ptr::null_mut(),
        wo_fen: 0,
        wo_fen_save: 0,
        wo_fdi: ptr::null_mut(),
        wo_fdl: 0,
        wo_fdl_save: 0,
        wo_fdm: ptr::null_mut(),
        wo_fdm_save: ptr::null_mut(),
        wo_fml: 0,
        wo_fdn: 0,
        wo_fde: crate::eval::typval::Callback::NONE,
        wo_fdt: crate::eval::typval::Callback::NONE,
        wo_fmr: ptr::null_mut(),
        wo_lbr: 0,
        wo_list: 0,
        wo_nu: 0,
        wo_rnu: 0,
        wo_ve: ptr::null_mut(),
        wo_ve_flags: 0,
        wo_nuw: 0,
        wo_wfb: 0,
        wo_wfh: 0,
        wo_wfw: 0,
        wo_wp: 0,
        wo_pvw: 0,
        wo_lhi: 0,
        wo_rl: 0,
        wo_rlc: ptr::null_mut(),
        wo_scr: 0,
        wo_sms: 0,
        wo_spell: 0,
        wo_cuc: 0,
        wo_cul: 0,
        wo_culopt: ptr::null_mut(),
        wo_cc: ptr::null_mut(),
        wo_sbr: ptr::null_mut(),
        wo_stc: ptr::null_mut(),
        wo_stl: ptr::null_mut(),
        wo_wbr: ptr::null_mut(),
        wo_scb: 0,
        wo_diff_saved: 0,
        wo_scb_save: 0,
        wo_wrap: 0,
        wo_wrap_save: 0,
        wo_cocu: ptr::null_mut(),
        wo_cole: 0,
        wo_crb: 0,
        wo_crb_save: 0,
        wo_scl: ptr::null_mut(),
        wo_siso: 0,
        wo_so: 0,
        wo_sop: 0,
        wo_winhl: ptr::null_mut(),
        wo_lcs: ptr::null_mut(),
        wo_fcs: ptr::null_mut(),
        wo_winbl: 0,
        wo_wrap_flags: 0,
        wo_stl_flags: 0,
        wo_wbr_flags: 0,
        wo_fde_flags: 0,
        wo_fdt_flags: 0,
        wo_script_ctx: [crate::runtime::Sctx {
            sc_sid: 0,
            sc_seq: 0,
            sc_lnum: 0,
            sc_chan: 0,
        }; K_WIN_OPT_COUNT],
    };
}

impl Drop for Winopt {
    fn drop(&mut self) {
        crate::option::clear_winopt(self);
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct WinConfig {
    pub window: crate::api::tabpage::Window,
    pub bufpos: crate::cursor::Lpos,
    pub height: c_int,
    pub width: c_int,
    pub row: f64,
    pub col: f64,
    pub anchor: c_int,
    pub relative: c_int,
    pub external: bool,
    pub focusable: bool,
    pub mouse: bool,
    pub split: c_int,
    pub zindex: c_int,
    pub style: c_int,
    pub border: bool,
    pub shadow: bool,
    pub border_chars: [[c_char; 32]; 8],
    pub border_hl_ids: [c_int; 8],
    pub border_attr: [c_int; 8],
    pub title: bool,
    pub title_pos: c_int,
    pub title_chunks: crate::decoration::VirtText,
    pub title_width: c_int,
    pub footer: bool,
    pub footer_pos: c_int,
    pub footer_chunks: crate::decoration::VirtText,
    pub footer_width: c_int,
    pub noautocmd: bool,
    pub fixed: bool,
    pub hide: bool,
    pub _cmdline_offset: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Frame {
    pub fr_layout: c_char,
    pub fr_width: c_int,
    pub fr_newwidth: c_int,
    pub fr_height: c_int,
    pub fr_newheight: c_int,
    pub fr_parent: *mut Frame,
    pub fr_next: *mut Frame,
    pub fr_prev: *mut Frame,
    pub fr_child: *mut Frame,
    pub fr_win: *mut WinInfoInfo,
}

impl Frame {
    pub const fn new() -> Self {
        Self {
            fr_layout: 0,
            fr_width: 0,
            fr_newwidth: 0,
            fr_height: 0,
            fr_newheight: 0,
            fr_parent: ptr::null_mut(),
            fr_next: ptr::null_mut(),
            fr_prev: ptr::null_mut(),
            fr_child: ptr::null_mut(),
            fr_win: ptr::null_mut(),
        }
    }
}

#[repr(C)]
pub struct WinInfoInfo {
    pub handle: ObjHandle,
    pub w_buffer: *mut FileBuf,
    pub w_s_owned: Option<Box<crate::syntax::Synblock>>,
    pub w_ns_hl: c_int,
    pub w_ns_hl_winhl: c_int,
    pub w_ns_hl_active: c_int,
    pub w_ns_hl_attr: Option<std::rc::Rc<crate::highlight::NSHlAttr>>,
    pub w_ns_set: std::collections::BTreeSet<u32>,
    pub w_hl_id_normal: c_int,
    pub w_hl_attr_normal: c_int,
    pub w_hl_attr_normalnc: c_int,
    pub w_hl_needs_update: c_int,
    pub w_prev: *mut WinInfoInfo,
    pub w_next: *mut WinInfoInfo,
    pub w_locked: c_int,
    pub w_frame: *mut Frame,
    pub w_cursor: CursorPos,
    pub w_curswant: i32,
    pub w_set_curswant: bool,
    pub w_cursorline: i32,
    pub w_last_cursorline: i32,
    pub w_old_visual_mode: c_char,
    pub w_old_cursor_lnum: i32,
    pub w_old_cursor_fcol: i32,
    pub w_old_cursor_lcol: i32,
    pub w_old_visual_lnum: i32,
    pub w_old_visual_col: i32,
    pub w_old_curswant: i32,
    pub w_last_cursor_lnum_rnu: i32,
    pub w_p_lcs_chars: LcsChars,
    pub w_p_fcs_chars: FcsChars,
    pub w_topline: i32,
    pub w_topline_was_set: bool,
    pub w_topfill: c_int,
    pub w_old_topfill: c_int,
    pub w_botfill: bool,
    pub w_old_botfill: bool,
    pub w_leftcol: i32,
    pub w_skipcol: i32,
    pub w_last_topline: i32,
    pub w_last_topfill: c_int,
    pub w_last_leftcol: i32,
    pub w_last_skipcol: i32,
    pub w_last_width: c_int,
    pub w_last_height: c_int,
    pub w_winrow: c_int,
    pub w_height: c_int,
    pub w_prev_winrow: c_int,
    pub w_prev_height: c_int,
    pub w_status_height: c_int,
    pub w_winbar_height: c_int,
    pub w_wincol: c_int,
    pub w_width: c_int,
    pub w_hsep_height: c_int,
    pub w_vsep_width: c_int,
    pub w_save_cursor: PosSave,
    pub w_do_win_fix_cursor: bool,
    pub w_wantline: c_int,
    pub w_wantcol: c_int,
    pub w_winrow_off: c_int,
    pub w_wincol_off: c_int,
    pub w_view_height: c_int,
    pub w_view_width: c_int,
    pub w_height_request: c_int,
    pub w_width_request: c_int,
    pub w_border_adj: [c_int; 4],
    pub w_height_outer: c_int,
    pub w_width_outer: c_int,
    pub w_valid: c_int,
    pub w_valid_cursor: CursorPos,
    pub w_valid_leftcol: i32,
    pub w_valid_skipcol: i32,
    pub w_viewport_invalid: bool,
    pub w_viewport_last_topline: i32,
    pub w_viewport_last_botline: i32,
    pub w_viewport_last_topfill: i32,
    pub w_viewport_last_skipcol: i32,
    pub w_cline_height: c_int,
    pub w_cline_folded: bool,
    pub w_cline_row: c_int,
    pub w_virtcol: i32,
    pub w_wrow: c_int,
    pub w_wcol: c_int,
    pub w_wcol_conceal_off: c_int,
    pub w_botline: i32,
    pub w_empty_rows: c_int,
    pub w_filler_rows: c_int,
    pub w_lines_valid: c_int,
    pub w_lines: Vec<crate::grid::Wline>,
    pub w_folds: Vec<FoldEntry>,
    pub w_fold_manual: bool,
    pub w_foldinvalid: bool,
    pub w_nrwidth: c_int,
    pub w_scwidth: c_int,
    pub w_minscwidth: c_int,
    pub w_maxscwidth: c_int,
    pub w_redr_type: c_int,
    pub w_upd_rows: c_int,
    pub w_redraw_top: i32,
    pub w_redraw_bot: i32,
    pub w_redr_status: bool,
    pub w_redr_border: bool,
    pub w_redr_statuscol: bool,
    pub w_display_tick: u64,
    pub w_stl_cursor: CursorPos,
    pub w_stl_virtcol: i32,
    pub w_stl_topline: i32,
    pub w_stl_line_count: i32,
    pub w_stl_topfill: c_int,
    pub w_stl_empty: c_char,
    pub w_stl_recording: c_int,
    pub w_stl_state: c_int,
    pub w_stl_visual_mode: c_int,
    pub w_stl_visual_pos: CursorPos,
    pub w_alt_fnum: c_int,
    pub w_alist: Option<std::rc::Rc<std::cell::RefCell<crate::arglist::Alist>>>,
    pub w_arg_idx: c_int,
    pub w_arg_idx_invalid: bool,
    pub w_localdir: Option<std::ffi::CString>,
    pub w_prevdir: Option<std::ffi::CString>,
    pub w_onebuf_opt: Winopt,
    pub w_allbuf_opt: Winopt,
    pub w_p_cc_cols: Vec<c_int>,
    pub w_p_culopt_flags: u8,
    pub w_briopt_min: c_int,
    pub w_briopt_shift: c_int,
    pub w_briopt_sbr: bool,
    pub w_briopt_list: c_int,
    pub w_briopt_vcol: c_int,
    pub w_scbind_pos: c_int,
    pub w_winvar: crate::eval::typval::ScopeDictDictItem,
    pub w_pcmark: CursorPos,
    pub w_prev_pcmark: CursorPos,
    pub w_jumplist: [crate::mark::Xfmark; 100],
    pub w_jumplistlen: c_int,
    pub w_jumplistidx: c_int,
    pub w_changelistidx: c_int,
    pub w_match_head: Vec<crate::r#match::Matchitem>,
    pub w_next_match_id: c_int,
    pub w_tagstack: [crate::tag::Taggy; 20],
    pub w_tagstackidx: c_int,
    pub w_tagstacklen: c_int,
    pub w_grid: GridView,
    pub w_grid_alloc: ScreenGrid,
    pub w_pos_changed: bool,
    pub w_floating: bool,
    pub w_kind: c_int,
    pub w_config: WinConfig,
    pub w_fraction: c_int,
    pub w_prev_fraction_row: c_int,
    pub w_nrwidth_line_count: i32,
    pub w_statuscol_line_count: i32,
    pub w_nrwidth_width: c_int,
    pub w_llist: *mut crate::quickfix::QfInfo,
    pub w_llist_ref: *mut crate::quickfix::QfInfo,
    pub w_status_click_defs: Vec<crate::mouse::StlClickDefinition>,
    pub w_winbar_click_defs: Vec<crate::mouse::StlClickDefinition>,
    pub w_statuscol_click_defs: std::collections::BTreeMap<c_int, StcClicks>,
}

impl WinInfoInfo {
    #[inline]
    pub fn buf_mut(&self) -> &'static mut FileBuf {
        unsafe { &mut *self.w_buffer }
    }

    pub const fn new() -> Self {
        const ZERO_POS: CursorPos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        Self {
            handle: 0,
            w_buffer: ptr::null_mut(),
            w_s_owned: None,
            w_ns_hl: 0,
            w_ns_hl_winhl: 0,
            w_ns_hl_active: 0,
            w_ns_hl_attr: None,
            w_ns_set: std::collections::BTreeSet::new(),
            w_hl_id_normal: 0,
            w_hl_attr_normal: 0,
            w_hl_attr_normalnc: 0,
            w_hl_needs_update: 0,
            w_prev: ptr::null_mut(),
            w_next: ptr::null_mut(),
            w_locked: 0,
            w_frame: ptr::null_mut(),
            w_cursor: CursorPos {
                lnum: 1,
                col: 0,
                coladd: 0,
            },
            w_curswant: 0,
            w_set_curswant: false,
            w_cursorline: 0,
            w_last_cursorline: 0,
            w_old_visual_mode: 0,
            w_old_cursor_lnum: 0,
            w_old_cursor_fcol: 0,
            w_old_cursor_lcol: 0,
            w_old_visual_lnum: 0,
            w_old_visual_col: 0,
            w_old_curswant: 0,
            w_last_cursor_lnum_rnu: 0,
            w_p_lcs_chars: LcsChars::new(),
            w_p_fcs_chars: FcsChars {
                stl: 0,
                stlnc: 0,
                wbr: 0,
                horiz: 0,
                horizup: 0,
                horizdown: 0,
                vert: 0,
                vertleft: 0,
                vertright: 0,
                verthoriz: 0,
                fold: 0,
                foldopen: 0,
                foldclosed: 0,
                foldsep: 0,
                foldinner: 0,
                diff: 0,
                msgsep: 0,
                eob: 0,
                lastline: 0,
                trunc: 0,
                truncrl: 0,
            },
            w_topline: 1,
            w_topline_was_set: false,
            w_topfill: 0,
            w_old_topfill: 0,
            w_botfill: false,
            w_old_botfill: false,
            w_leftcol: 0,
            w_skipcol: 0,
            w_last_topline: 0,
            w_last_topfill: 0,
            w_last_leftcol: 0,
            w_last_skipcol: 0,
            w_last_width: 0,
            w_last_height: 0,
            w_winrow: 0,
            w_height: 0,
            w_prev_winrow: 0,
            w_prev_height: 0,
            w_status_height: 0,
            w_winbar_height: 0,
            w_wincol: 0,
            w_width: 0,
            w_hsep_height: 0,
            w_vsep_width: 0,
            w_save_cursor: PosSave {
                w_topline_save: 0,
                w_topline_corr: 0,
                w_cursor_save: ZERO_POS,
                w_cursor_corr: ZERO_POS,
            },
            w_do_win_fix_cursor: false,
            w_wantline: 0,
            w_wantcol: 0,
            w_winrow_off: 0,
            w_wincol_off: 0,
            w_view_height: 0,
            w_view_width: 0,
            w_height_request: 0,
            w_width_request: 0,
            w_border_adj: [0; 4],
            w_height_outer: 0,
            w_width_outer: 0,
            w_valid: 0,
            w_valid_cursor: ZERO_POS,
            w_valid_leftcol: 0,
            w_valid_skipcol: 0,
            w_viewport_invalid: true,
            w_viewport_last_topline: 1,
            w_viewport_last_botline: 0,
            w_viewport_last_topfill: 0,
            w_viewport_last_skipcol: 0,
            w_cline_height: 0,
            w_cline_folded: false,
            w_cline_row: 0,
            w_virtcol: 0,
            w_wrow: 0,
            w_wcol: 0,
            w_wcol_conceal_off: 0,
            w_botline: 2,
            w_empty_rows: 0,
            w_filler_rows: 0,
            w_lines_valid: 0,
            w_lines: Vec::new(),
            w_folds: Vec::new(),
            w_fold_manual: false,
            w_foldinvalid: false,
            w_nrwidth: 0,
            w_scwidth: 0,
            w_minscwidth: 0,
            w_maxscwidth: 0,
            w_redr_type: 0,
            w_upd_rows: 0,
            w_redraw_top: 0,
            w_redraw_bot: 0,
            w_redr_status: false,
            w_redr_border: false,
            w_redr_statuscol: false,
            w_display_tick: 0,
            w_stl_cursor: ZERO_POS,
            w_stl_virtcol: 0,
            w_stl_topline: 0,
            w_stl_line_count: 0,
            w_stl_topfill: 0,
            w_stl_empty: 0,
            w_stl_recording: 0,
            w_stl_state: 0,
            w_stl_visual_mode: 0,
            w_stl_visual_pos: ZERO_POS,
            w_alt_fnum: 0,
            w_alist: None,
            w_arg_idx: 0,
            w_arg_idx_invalid: false,
            w_localdir: None,
            w_prevdir: None,
            w_onebuf_opt: Winopt::ZERO,
            w_allbuf_opt: Winopt::ZERO,
            w_p_cc_cols: Vec::new(),
            w_p_culopt_flags: 0,
            w_briopt_min: 0,
            w_briopt_shift: 0,
            w_briopt_sbr: false,
            w_briopt_list: 0,
            w_briopt_vcol: 0,
            w_scbind_pos: 1,
            w_winvar: crate::eval::typval::ScopeDictDictItem::new(),
            w_pcmark: ZERO_POS,
            w_prev_pcmark: ZERO_POS,
            w_jumplist: [const { crate::mark::Xfmark::ZERO }; 100],
            w_jumplistlen: 0,
            w_jumplistidx: 0,
            w_changelistidx: 0,
            w_match_head: Vec::new(),
            w_next_match_id: 0,
            w_tagstack: [const { crate::tag::Taggy::ZERO }; 20],
            w_tagstackidx: 0,
            w_tagstacklen: 0,
            w_grid: GridView {
                target: ptr::null_mut(),
                row_offset: 0,
                col_offset: 0,
            },
            w_grid_alloc: SCREEN_GRID_INIT,
            w_pos_changed: false,
            w_floating: false,
            w_kind: K_WIN_NORMAL,
            w_config: WIN_CONFIG_INIT,
            w_fraction: 0,
            w_prev_fraction_row: 0,
            w_nrwidth_line_count: 0,
            w_statuscol_line_count: 0,
            w_nrwidth_width: 0,
            w_llist: ptr::null_mut(),
            w_llist_ref: ptr::null_mut(),
            w_status_click_defs: Vec::new(),
            w_winbar_click_defs: Vec::new(),
            w_statuscol_click_defs: std::collections::BTreeMap::new(),
        }
    }

    #[inline]
    pub fn w_s_ptr(&self) -> &'static mut crate::syntax::Synblock {
        unsafe {
            if let Some(ref owned) = self.w_s_owned {
                &mut *(&raw const **owned).cast_mut()
            } else {
                &mut (*self.w_buffer).b_s
            }
        }
    }

    #[inline]
    pub fn w_vars(&self) -> &'static mut TvDict {
        unsafe { &mut *self.w_winvar.di_tv.vval.v_dict() }
    }
}

pub type StcClick = Vec<crate::mouse::StlClickDefinition>;
pub type StcClicks = std::collections::BTreeMap<c_int, StcClick>;

pub struct SnapshotFrame {
    pub fr_layout: c_char,
    pub fr_width: c_int,
    pub fr_height: c_int,
    pub fr_next: Option<Box<SnapshotFrame>>,
    pub fr_child: Option<Box<SnapshotFrame>>,
    pub fr_win: *mut WinInfoInfo,
}

#[repr(C)]
pub struct TabPageInfo {
    pub handle: ObjHandle,
    pub tp_next: *mut TabPageInfo,
    pub tp_topframe: *mut Frame,
    pub tp_curwin: *mut WinInfoInfo,
    pub tp_prevwin: *mut WinInfoInfo,
    pub tp_firstwin: *mut WinInfoInfo,
    pub tp_lastwin: *mut WinInfoInfo,
    pub tp_old_rows_avail: i64,
    pub tp_old_columns: i64,
    pub tp_ch_used: i64,
    pub tp_did_tabclosedpre: bool,
    pub tp_diffs: Vec<crate::diff::DiffBlock>,
    pub tp_diffbuf: [*mut FileBuf; 8],
    pub tp_diff_invalid: c_int,
    pub tp_diff_update: c_int,
    pub tp_snapshot: [Option<Box<SnapshotFrame>>; 3],
    pub tp_winvar: crate::eval::typval::ScopeDictDictItem,
    pub tp_localdir: Option<std::ffi::CString>,
    pub tp_prevdir: Option<std::ffi::CString>,
}

impl TabPageInfo {
    pub const fn new() -> Self {
        Self {
            handle: 0,
            tp_next: ptr::null_mut(),
            tp_topframe: ptr::null_mut(),
            tp_curwin: ptr::null_mut(),
            tp_prevwin: ptr::null_mut(),
            tp_firstwin: ptr::null_mut(),
            tp_lastwin: ptr::null_mut(),
            tp_old_rows_avail: 0,
            tp_old_columns: 0,
            tp_ch_used: 0,
            tp_did_tabclosedpre: false,
            tp_diffs: Vec::new(),
            tp_diffbuf: [ptr::null_mut(); 8],
            tp_diff_invalid: 1,
            tp_diff_update: 0,
            tp_snapshot: [None, None, None],
            tp_winvar: crate::eval::typval::ScopeDictDictItem::new(),
            tp_localdir: None,
            tp_prevdir: None,
        }
    }

    #[inline]
    pub fn tp_vars(&self) -> &'static mut TvDict {
        unsafe { &mut *self.tp_winvar.di_tv.vval.v_dict() }
    }
}

use crate::api::vim::K_CD_SCOPE_GLOBAL;
use crate::api::window::K_CTX_KEEP_CWD;
use crate::buffer::{BCO_ENTER, BCO_NOHELP};
use crate::charset::NUL;
use crate::drawline::{VALID_CROW, VALID_WCOL};
use crate::eval::fs::{K_CD_SCOPE_BUFFER, K_CD_SCOPE_TABPAGE, K_CD_SCOPE_WINDOW};
use crate::eval::typval::VAR_UNLOCKED;
use crate::ex_docmd::CMOD_CONFIRM;
use crate::insexpand::CTRL_I_CONST;
use crate::os::fs::MAXPATHL;
use crate::regexp::{CTRL_H_CONST, CTRL_V_CONST, MAXCOL, NUMBUFLEN};
use crate::runtime::SID_WINLAYOUT;
use crate::search::FORWARD;

pub const WSP_ROOM: c_int = 0x01;
pub const WSP_VERT: c_int = 0x02;
pub const WSP_HOR: c_int = 0x04;
pub const WSP_TOP: c_int = 0x08;
pub const WSP_BOT: c_int = 0x10;
pub const WSP_HELP: c_int = 0x20;
pub const WSP_BELOW: c_int = 0x40;
pub const WSP_ABOVE: c_int = 0x80;
pub const WSP_NEWLOC: c_int = 0x100;
pub const WSP_NOENTER: c_int = 0x200;
pub const WSP_QUICKFIX: c_int = 0x400;

pub const MIN_COLUMNS: c_int = 12;
pub const MIN_LINES: c_int = 2;
pub const STATUS_HEIGHT: c_int = 1;
pub const LOWEST_WIN_ID: c_int = 1000;

const SNAP_HELP_IDX: usize = 0;
const SNAP_QUICKFIX_IDX: usize = 2;
const SNAP_COUNT: usize = 3;

const FRACTION_MULT: c_int = 16384;

const WEE_UNDO_SYNC: c_int = 0x01;
const WEE_CURWIN_INVALID: c_int = 0x02;
const WEE_TRIGGER_NEW_AUTOCMDS: c_int = 0x04;
const WEE_TRIGGER_ENTER_AUTOCMDS: c_int = 0x08;
const WEE_TRIGGER_LEAVE_AUTOCMDS: c_int = 0x10;

const K_OPT_TCL_FLAG_LEFT: c_uint = 0x01;
const K_OPT_TCL_FLAG_USELAST: c_uint = 0x02;

const K_WIN_OPT_SCROLL: usize = 33;

const K_WIN_NORMAL: WinKind = 0;

const E_CANNOT_CLOSE_LAST_WINDOW: &CStr = c"E444: Cannot close last window";
const E_CANNOT_SPLIT_WINDOW_WHEN_CLOSING_BUFFER: &CStr =
    c"E1159: Cannot split a window when closing the buffer";
const M_ONLYONE: &CStr = c"Already only one window";

struct WindowModuleState {
    split_disallowed: c_int,
    close_disallowed: c_int,
    frame_locked: c_int,
    min_set_ch: OptInt,
    command_frame_height: bool,
    last_tp_handle: c_int,
    last_win_id: c_int,
    old_rows: c_int,
    old_columns: c_int,
    did_initial_scroll_size_snapshot: bool,
    winclosed_recursive: bool,
    tabclosedpre_recursive: bool,
    win_scrolled_resized_recursive: bool,
}

impl WindowModuleState {
    const fn new() -> Self {
        Self {
            split_disallowed: 0,
            close_disallowed: 0,
            frame_locked: 0,
            min_set_ch: 1,
            command_frame_height: true,
            last_tp_handle: 0,
            last_win_id: LOWEST_WIN_ID - 1,
            old_rows: 0,
            old_columns: 0,
            did_initial_scroll_size_snapshot: false,
            winclosed_recursive: false,
            tabclosedpre_recursive: false,
            win_scrolled_resized_recursive: false,
        }
    }
}

static WIN_STATE: GlobalState<WindowModuleState> = GlobalState::new(WindowModuleState::new());

#[inline]
fn win_st() -> &'static mut WindowModuleState {
    unsafe { &mut *WIN_STATE.as_ptr() }
}

const ERROR_INIT: Error = Error::new();

#[inline]
fn nowin() -> *mut WinInfoInfo {
    (-1isize) as *mut WinInfoInfo
}

#[inline]
fn rows_avail() -> OptInt {
    OptInt::from(g_st().rows_global)
        - opt_vars().p_ch
        - OptInt::from(tabline_height())
        - OptInt::from(global_stl_height())
}

#[inline]
fn one_window_macro() -> bool {
    ptr::eq(g_st().firstwin, g_st().lastwin)
}

#[inline]
pub fn reset_binding(wp: &mut WinInfoInfo) {
    wp.w_onebuf_opt.wo_scb = 0;
    wp.w_onebuf_opt.wo_crb = 0;
}

/// Disallow changing the window layout (split window, close window, move
/// window). Resizing is still allowed.
pub fn window_layout_lock() {
    win_st().split_disallowed += 1;
    win_st().close_disallowed += 1;
}

pub fn window_layout_unlock() {
    win_st().split_disallowed -= 1;
    win_st().close_disallowed -= 1;
}

pub fn frames_locked() -> bool {
    win_st().frame_locked != 0
}

/// When the window layout cannot be changed give an error and return true.
pub fn window_layout_locked(cmd: Cmdidx) -> bool {
    let mut err = ERROR_INIT;
    let locked = window_layout_locked_err(cmd, &mut err);
    if error_set(&err) {
        emsg(gettext(err.msg_ptr()));
        api_clear_error(&mut err);
    }
    locked
}

/// Like `window_layout_locked`, but set `err` to the (untranslated) error message when locked.
pub fn window_layout_locked_err(cmd: Cmdidx, err: &mut Error) -> bool {
    unsafe {
        if win_st().split_disallowed > 0 || win_st().close_disallowed > 0 {
            if win_st().close_disallowed == 0 && cmd == Cmdidx::CmdTabnew {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"%s",
                    E_CANNOT_SPLIT_WINDOW_WHEN_CLOSING_BUFFER,
                );
            } else {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"%s",
                    E_NOT_ALLOWED_TO_CHANGE_WINDOW_LAYOUT_IN_THIS_AUTOCMD,
                );
            }
            return true;
        }
        false
    }
}

/// Check if the current window is allowed to move to a different buffer.
pub fn check_can_set_curbuf_disabled() -> bool {
    {
        if curwin_mut().w_onebuf_opt.wo_wfb != 0 {
            emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
            return false;
        }
        true
    }
}

/// Check if the current window is allowed to move to a different buffer.
pub fn check_can_set_curbuf_forceit(forceit: c_int) -> bool {
    {
        if forceit == 0 && curwin_mut().w_onebuf_opt.wo_wfb != 0 {
            emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
            return false;
        }
        true
    }
}

/// Return the current window, unless in the cmdline window and "prevwin" is
/// set, then return "prevwin".
pub fn prevwin_curwin() -> &'static mut WinInfoInfo {
    unsafe {
        if is_in_cmdwin() && !g_st().prevwin.is_null() {
            &mut *g_st().prevwin
        } else {
            curwin_mut()
        }
    }
}

/// If the 'switchbuf' option contains "useopen" or "usetab", then try to jump
/// to a window containing "buf".
/// Returns the pointer to the window that was jumped to or NULL.
pub fn swbuf_goto_win_with_buf(buf: *mut FileBuf) -> *mut WinInfoInfo {
    let mut wp: *mut WinInfoInfo = ptr::null_mut();

    if buf.is_null() {
        return wp;
    }

    if (opt_vars().swb_flags & K_OPT_SWB_FLAG_USEOPEN) != 0 {
        wp = buf_jump_open_win(buf);
    }

    if wp.is_null() && (opt_vars().swb_flags & K_OPT_SWB_FLAG_USETAB) != 0 {
        wp = buf_jump_open_tab(buf);
    }

    wp
}

#[allow(non_snake_case)]
fn do_window_newwindow(nchar: c_int, Prenum: c_int, cbuf: &mut [c_char; 40]) {
    unsafe {
        if Prenum != 0 {
            vim_snprintf!(
                cbuf.as_mut_ptr(),
                cbuf.len() - 5,
                c"%lld",
                i64::from(Prenum),
            );
        } else {
            cbuf[0] = NUL;
        }
        if nchar == b'v' as c_int || nchar == CTRL_V_CONST {
            {
                let _dst: *mut c_char = cbuf.as_mut_ptr();
                let _dsize: usize = cbuf.len();
                let _dlen = crate::memory::cstr_len(_dst);
                crate::memory::xstrlcpy(_dst.add(_dlen), c"v", _dsize - _dlen);
            };
        }
        {
            let _dst: *mut c_char = cbuf.as_mut_ptr();
            let _dsize: usize = cbuf.len();
            let _dlen = crate::memory::cstr_len(_dst);
            crate::memory::xstrlcpy(_dst.add(_dlen), c"new", _dsize - _dlen);
        };
        do_cmdline_cmd(cbuf.as_ptr());
    }
}

fn do_window_gotofile(nchar: c_int, prenum1: c_int) {
    unsafe {
        if check_text_or_curbuf_locked(None) {
            return;
        }

        let mut lnum: Linenr = -1;
        if let Some(fname_cstr) = grab_file_name(prenum1, &mut lnum) {
            let ptr = fname_cstr.as_ptr().cast_mut();
            let oldtab = g_st().curtab;
            let oldwin = curwin_mut();
            setpcmark();

            let mut wp: *mut WinInfoInfo = ptr::null_mut();
            if (opt_vars().swb_flags & (K_OPT_SWB_FLAG_USEOPEN | K_OPT_SWB_FLAG_USETAB)) != 0
                && g_st().cmdmod.cmod_tab == 0
            {
                wp = swbuf_goto_win_with_buf(buflist_findname_exp(ptr));
            }

            if wp.is_null() && win_split(0, 0) {
                reset_binding(curwin_mut());
                if !do_ecmd(
                    0,
                    ptr,
                    ptr::null_mut(),
                    None,
                    ECMD_LASTL,
                    ECMD_HIDE,
                    ptr::null_mut(),
                ) {
                    let old_got_int = g_st().got_int;
                    g_st().got_int = false;

                    win_close(curwin_ptr(), false, false);
                    g_st().got_int = g_st().got_int || old_got_int;

                    goto_tabpage_win(&mut *oldtab, oldwin);
                } else {
                    wp = curwin_ptr();
                }
            }

            if !wp.is_null() && nchar == b'F' as c_int && lnum >= 0 {
                curwin_mut().w_cursor.lnum = lnum;
                check_cursor_lnum(curwin_mut());
                beginline(BL_SOL | BL_FIX);
            }
        }
    }
}

/// All CTRL-W window commands are handled here, called from normal_cmd().
pub fn do_window(mut nchar: c_int, mut prenum: c_int, mut xchar: c_int) {
    unsafe {
        let mut type_ = FIND_DEFINE;
        let mut cbuf = [0 as c_char; 40];
        let prenum1 = if prenum == 0 { 1 } else { prenum };

        match nchar {
            c if c == b'S' as c_int || c == CTRL_S_CONST || c == b's' as c_int => {
                reset_visual_and_resel();
                if bt_quickfix(Some(crate::globals::curbuf_mut())) {
                    do_window_newwindow(nchar, prenum, &mut cbuf);
                } else {
                    win_split(prenum, 0);
                }
            }
            c if c == CTRL_V_CONST || c == b'v' as c_int => {
                reset_visual_and_resel();
                if bt_quickfix(Some(crate::globals::curbuf_mut())) {
                    do_window_newwindow(nchar, prenum, &mut cbuf);
                } else {
                    win_split(prenum, WSP_VERT);
                }
            }
            c if c == CTRL_HAT_CONST || c == b'^' as c_int => {
                reset_visual_and_resel();

                if buflist_findnr(if prenum == 0 {
                    curwin_mut().w_alt_fnum
                } else {
                    prenum
                })
                .is_null()
                {
                    if prenum == 0 {
                        emsg(gettext(E_NOALT));
                    } else {
                        semsg!(gettext(E_BUFFER_NR_NOT_FOUND), i64::from(prenum));
                    }
                    return;
                }

                if !curbuf_locked() && win_split(0, 0) {
                    buflist_getfile(
                        if prenum == 0 {
                            curwin_mut().w_alt_fnum
                        } else {
                            prenum
                        },
                        0,
                        GETF_ALT,
                        0,
                    );
                }
            }
            c if c == CTRL_N || c == b'n' as c_int => {
                reset_visual_and_resel();
                do_window_newwindow(nchar, prenum, &mut cbuf);
            }
            c if c == CTRL_Q_CONST || c == b'q' as c_int => {
                reset_visual_and_resel();
                cmd_with_count(c"quit", cbuf.as_mut_ptr(), cbuf.len(), i64::from(prenum));
                do_cmdline_cmd(cbuf.as_ptr());
            }
            c if c == CTRL_C_CONST || c == b'c' as c_int => {
                reset_visual_and_resel();
                cmd_with_count(c"close", cbuf.as_mut_ptr(), cbuf.len(), i64::from(prenum));
                do_cmdline_cmd(cbuf.as_ptr());
            }
            c if c == CTRL_Z_CONST || c == b'z' as c_int => {
                reset_visual_and_resel();
                do_cmdline_cmd(c"pclose");
            }
            c if c == b'P' as c_int => {
                let mut wp: *mut WinInfoInfo = ptr::null_mut();
                for wp2 in for_all_windows_ptr() {
                    if (*wp2).w_onebuf_opt.wo_pvw != 0 {
                        wp = wp2;
                        break;
                    }
                }
                if wp.is_null() {
                    emsg(gettext(c"E441: There is no preview window"));
                } else {
                    win_goto(wp);
                }
            }
            c if c == CTRL_O || c == b'o' as c_int => {
                reset_visual_and_resel();
                cmd_with_count(c"only", cbuf.as_mut_ptr(), cbuf.len(), i64::from(prenum));
                do_cmdline_cmd(cbuf.as_ptr());
            }
            c if c == CTRL_W_CONST || c == b'w' as c_int || c == b'W' as c_int => {
                if one_window_macro() && prenum != 1 {
                    beep_flush();
                } else {
                    let mut wp: *mut WinInfoInfo;
                    if prenum != 0 {
                        let mut last_focusable = g_st().firstwin;
                        wp = g_st().firstwin;
                        while {
                            prenum -= 1;
                            prenum > 0
                        } {
                            if !(*wp).w_floating
                                || (!(*wp).w_config.hide && (*wp).w_config.focusable)
                            {
                                last_focusable = wp;
                            }
                            if (*wp).w_next.is_null() {
                                break;
                            }
                            wp = (*wp).w_next;
                        }
                        while !wp.is_null()
                            && (*wp).w_floating
                            && ((*wp).w_config.hide || !(*wp).w_config.focusable)
                        {
                            wp = (*wp).w_next;
                        }
                        if wp.is_null() {
                            wp = last_focusable;
                        }
                    } else if nchar == b'W' as c_int {
                        wp = curwin_mut().w_prev;
                        if wp.is_null() {
                            wp = g_st().lastwin;
                        }
                        while !wp.is_null()
                            && (*wp).w_floating
                            && ((*wp).w_config.hide || !(*wp).w_config.focusable)
                        {
                            wp = (*wp).w_prev;
                        }
                    } else {
                        wp = curwin_mut().w_next;
                        while !wp.is_null()
                            && (*wp).w_floating
                            && ((*wp).w_config.hide || !(*wp).w_config.focusable)
                        {
                            wp = (*wp).w_next;
                        }
                        if wp.is_null() {
                            wp = g_st().firstwin;
                        }
                    }
                    win_goto(wp);
                }
            }
            c if c == b'j' as c_int || c == K_DOWN || c == CTRL_J_CONST => {
                win_goto_ver(false, prenum1);
            }
            c if c == b'k' as c_int || c == K_UP || c == CTRL_K_CONST => {
                win_goto_ver(true, prenum1);
            }
            c if c == b'h' as c_int || c == K_LEFT || c == CTRL_H_CONST || c == K_BS => {
                win_goto_hor(true, prenum1);
            }
            c if c == b'l' as c_int || c == K_RIGHT || c == CTRL_L_CONST => {
                win_goto_hor(false, prenum1);
            }
            c if c == b'T' as c_int => {
                if one_window(curwin_mut(), ptr::null_mut()) {
                    msg(gettext(M_ONLYONE), 0);
                } else {
                    let oldtab = g_st().curtab;
                    let wp = curwin_mut();
                    if !win_new_tabpage(prenum, ptr::null_mut(), true, ptr::null_mut()).is_null()
                        && valid_tabpage(oldtab)
                    {
                        let newtab = g_st().curtab;
                        goto_tabpage_tp(oldtab, true, true);
                        if ptr::eq(curwin_ptr(), wp) {
                            win_close(curwin_ptr(), false, false);
                        }
                        if valid_tabpage(newtab) {
                            goto_tabpage_tp(newtab, true, true);
                            apply_autocmds(
                                EVENT_TABNEWENTERED,
                                ptr::null_mut(),
                                ptr::null_mut(),
                                false,
                                g_st().curbuf,
                            );
                        }
                    }
                }
            }
            c if c == b't' as c_int || c == CTRL_T_CONST => {
                win_goto(g_st().firstwin);
            }
            c if c == b'b' as c_int || c == CTRL_B_CONST => {
                win_goto(lastwin_nofloating(ptr::null_mut()));
            }
            c if c == b'p' as c_int || c == CTRL_P => {
                if !win_valid(g_st().prevwin)
                    || (*g_st().prevwin).w_config.hide
                    || !(*g_st().prevwin).w_config.focusable
                {
                    beep_flush();
                } else {
                    win_goto(g_st().prevwin);
                }
            }
            c if c == b'x' as c_int || c == CTRL_X_CONST => {
                win_exchange(prenum);
            }
            c if c == CTRL_R_CONST || c == b'r' as c_int => {
                reset_visual_and_resel();
                win_rotate(false, prenum1);
            }
            c if c == b'R' as c_int => {
                reset_visual_and_resel();
                win_rotate(true, prenum1);
            }
            c if c == b'K' as c_int
                || c == b'J' as c_int
                || c == b'H' as c_int
                || c == b'L' as c_int =>
            {
                if one_window(curwin_mut(), ptr::null_mut()) {
                    beep_flush();
                } else {
                    let dir = (if nchar == b'H' as c_int || nchar == b'L' as c_int {
                        WSP_VERT
                    } else {
                        0
                    }) | (if nchar == b'H' as c_int || nchar == b'K' as c_int {
                        WSP_TOP
                    } else {
                        WSP_BOT
                    });
                    win_splitmove(curwin_mut(), prenum, dir);
                }
            }
            c if c == b'=' as c_int => {
                let mod_ = g_st().cmdmod.cmod_split & (WSP_VERT | WSP_HOR);
                win_equal(
                    ptr::null_mut(),
                    false,
                    if mod_ == WSP_VERT {
                        b'v' as c_int
                    } else if mod_ == WSP_HOR {
                        b'h' as c_int
                    } else {
                        b'b' as c_int
                    },
                );
            }
            c if c == b'+' as c_int => {
                win_setheight(curwin_mut().w_height + prenum1);
            }
            c if c == b'-' as c_int => {
                win_setheight(curwin_mut().w_height - prenum1);
            }
            c if c == CTRL_CONST_CONST || c == b'_' as c_int => {
                win_setheight(if prenum != 0 {
                    prenum
                } else {
                    g_st().rows_global - win_st().min_set_ch as c_int
                });
            }
            c if c == b'>' as c_int => {
                win_setwidth(curwin_mut().w_width + prenum1);
            }
            c if c == b'<' as c_int => {
                win_setwidth(curwin_mut().w_width - prenum1);
            }
            c if c == b'|' as c_int => {
                win_setwidth(if prenum != 0 {
                    prenum
                } else {
                    g_st().columns_global
                });
            }
            c if c == b'}' as c_int || c == b']' as c_int || c == CTRL_RSB_CONST => {
                if nchar == b'}' as c_int {
                    if prenum != 0 {
                        g_st().g_do_tagpreview = prenum;
                    } else {
                        g_st().g_do_tagpreview = opt_vars().p_pvh as c_int;
                    }
                } else {
                    g_st().g_do_tagpreview = 0;
                }

                if prenum != 0 {
                    g_st().postponed_split = prenum;
                } else {
                    g_st().postponed_split = -1;
                }

                do_nv_ident(CTRL_RSB_CONST, NUL as c_int);
                g_st().postponed_split = 0;
            }
            c if c == b'f' as c_int || c == b'F' as c_int || c == CTRL_F_CONST => {
                do_window_gotofile(nchar, prenum1);
            }
            c if c == b'i' as c_int
                || c == CTRL_I_CONST
                || c == b'd' as c_int
                || c == CTRL_D_CONST =>
            {
                if nchar == b'i' as c_int || nchar == CTRL_I_CONST {
                    type_ = FIND_ANY;
                }
                let mut ptr: *mut c_char = ptr::null_mut();
                let len = find_ident_under_cursor(&raw mut ptr, FIND_IDENT, None);
                if len == 0 {
                    return;
                }
                let pat = std::ffi::CString::new(crate::memory::cstr_slice(ptr, len)).unwrap();
                find_pattern_in_path(
                    pat.as_ptr().cast_mut(),
                    0,
                    len,
                    true,
                    prenum == 0,
                    type_,
                    prenum1,
                    ACTION_SPLIT,
                    1,
                    MAXLNUM,
                    false,
                    false,
                );
                curwin_mut().w_set_curswant = true;
            }
            c if c == K_KENTER || c == CAR => {
                if bt_quickfix(Some(crate::globals::curbuf_mut())) {
                    qf_view_result(true);
                }
            }
            c if c == b'g' as c_int || c == CTRL_G => {
                g_st().no_mapping += 1;
                g_st().allow_keys += 1;
                if xchar == NUL as c_int {
                    xchar = plain_vgetc();
                }
                langmap_adjust(&mut xchar, true);
                g_st().no_mapping -= 1;
                g_st().allow_keys -= 1;
                add_to_showcmd(xchar);

                match xchar {
                    xc if xc == b'}' as c_int || xc == b']' as c_int || xc == CTRL_RSB_CONST => {
                        if xchar == b'}' as c_int {
                            xchar = CTRL_RSB_CONST;
                            if prenum != 0 {
                                g_st().g_do_tagpreview = prenum;
                            } else {
                                g_st().g_do_tagpreview = opt_vars().p_pvh as c_int;
                            }
                        }
                        if prenum != 0 {
                            g_st().postponed_split = prenum;
                        } else {
                            g_st().postponed_split = -1;
                        }
                        do_nv_ident(b'g' as c_int, xchar);
                        g_st().postponed_split = 0;
                    }
                    xc if xc == b'f' as c_int || xc == b'F' as c_int => {
                        g_st().cmdmod.cmod_tab = tabpage_index(g_st().curtab) + 1;
                        nchar = xchar;
                        do_window_gotofile(nchar, prenum1);
                    }
                    xc if xc == b't' as c_int => {
                        goto_tabpage(prenum);
                    }
                    xc if xc == b'T' as c_int => {
                        goto_tabpage(-prenum1);
                    }
                    xc if xc == TAB => {
                        if !goto_tabpage_lastused() {
                            beep_flush();
                        }
                    }
                    xc if xc == b'e' as c_int => {
                        if curwin_mut().w_floating || !ui_has(K_UIMULTIGRID) {
                            beep_flush();
                            return;
                        }
                        let mut config = WIN_CONFIG_INIT;
                        config.width = curwin_mut().w_width;
                        config.height = curwin_mut().w_height;
                        config.external = true;
                        let mut err = ERROR_INIT;
                        if win_new_float(curwin_ptr(), false, config, &mut err).is_null() {
                            emsg(err.msg_ptr());
                            api_clear_error(&mut err);
                            beep_flush();
                        }
                    }
                    _ => {
                        beep_flush();
                    }
                }
            }
            _ => {
                beep_flush();
            }
        }
    }
}

fn cmd_with_count(cmd: &std::ffi::CStr, bufp: *mut c_char, bufsize: usize, prenum: i64) {
    unsafe {
        let len = xstrlcpy(bufp, cmd.as_ptr(), bufsize);
        if prenum > 0 && len < bufsize {
            vim_snprintf!(bufp.add(len), bufsize - len, c"%lld", prenum);
        }
    }
}

pub fn win_set_buf(win: &mut WinInfoInfo, buf: &mut FileBuf, err: &mut Error) {
    unsafe {
        let win_handle = win.handle;
        let tab = win_find_tabpage(&raw mut *win);

        g_st().redrawing_disabled += 1;

        let mut switchwin: CtxSwitch = CtxSwitch::new();

        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        let win_ok = ctx_switch(
            &mut switchwin,
            &raw mut *win,
            tab,
            ptr::null_mut(),
            K_CTX_NO_DISPLAY | K_CTX_KEEP_CWD,
        );
        if win_ok {
            let save_acd = opt_vars().p_acd;
            if !switchwin.cs_same_win {
                opt_vars().p_acd = 0;
            }

            do_buffer(DOBUF_GOTO, DOBUF_FIRST, FORWARD, buf.b_fnum, 0);

            if !switchwin.cs_same_win {
                opt_vars().p_acd = save_acd;
            }
        }
        try_leave(&tstate, err);

        if !win_ok && !error_set(err) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Failed to switch to window %d",
                win_handle,
            );
        }

        validate_cursor(curwin_mut());

        ctx_restore(&mut switchwin);
        g_st().redrawing_disabled -= 1;
    }
}

/// Return the number of fold columns to display
pub fn win_fdccol_count(wp: &mut WinInfoInfo) -> c_int {
    {
        let fdc = wp.w_onebuf_opt.wo_fdc;
        if crate::memory::cstr_first_byte(fdc) == b'a'
            && crate::memory::cstr_byte_at(fdc, 1) == b'u'
            && crate::memory::cstr_byte_at(fdc, 2) == b't'
            && crate::memory::cstr_byte_at(fdc, 3) == b'o'
        {
            let fdccol = if crate::memory::cstr_byte_at(fdc, 4) == b':' {
                c_int::from(crate::memory::cstr_byte_at(fdc, 5)) - c_int::from(b'0')
            } else {
                1
            };
            let needed_fdccols = get_deepest_nesting(wp);
            return fdccol.min(needed_fdccols);
        }
        c_int::from(crate::memory::cstr_first_byte(fdc)) - c_int::from(b'0')
    }
}

/// Merges two window configs, freeing replaced fields if necessary.
pub fn merge_win_config(dst: &mut WinConfig, src: WinConfig) {
    *dst = src;
}

/// Clear fields in `fconfig` that are only used for floating windows.
pub fn clear_float_config(fconfig: &mut WinConfig, free_fields: bool) {
    let saved_style = fconfig.style;
    let saved_cmdline_offset = fconfig._cmdline_offset;
    if free_fields {
        merge_win_config(fconfig, WIN_CONFIG_INIT);
    } else {
        *fconfig = WIN_CONFIG_INIT;
    }
    fconfig.style = saved_style;
    fconfig._cmdline_offset = saved_cmdline_offset;
}

pub fn ui_ext_win_position(wp: &mut WinInfoInfo, validate: bool) {
    unsafe {
        wp.w_pos_changed = false;
        if !wp.w_floating {
            if ui_has(K_UIMULTIGRID) {
                wp.w_grid_alloc.comp_col = wp.w_wincol;
                wp.w_grid_alloc.comp_row = wp.w_winrow;
            }
            ui_call_win_pos(
                Integer::from(wp.w_grid_alloc.handle),
                wp.handle,
                Integer::from(wp.w_winrow),
                Integer::from(wp.w_wincol),
                Integer::from(wp.w_width),
                Integer::from(wp.w_height),
            );
            return;
        }

        let c = &wp.w_config;
        if !c.external {
            let mut grid: *mut ScreenGrid = &raw mut g_st().default_grid;
            let mut row = c.row;
            let mut col = c.col;
            if c.relative == K_FLOAT_RELATIVE_WINDOW {
                let mut dummy = ERROR_INIT;
                let win = find_window_by_handle(c.window, &mut dummy);
                api_clear_error(&mut dummy);
                if !win.is_null() {
                    if (*win).w_pos_changed
                        && !(*win).w_grid_alloc.chars.is_empty()
                        && win_valid(win)
                    {
                        ui_ext_win_position(&mut *win, validate);
                    }
                    let mut row_off: c_int = 0;
                    let mut col_off: c_int = 0;
                    win_grid_alloc(&mut *win);
                    grid = grid_adjust(&mut (*win).w_grid, &mut row_off, &mut col_off);
                    row += f64::from(row_off);
                    col += f64::from(col_off);
                    if c.bufpos.lnum >= 0 {
                        let lnum = (c.bufpos.lnum + 1).min((*win).buf_mut().b_ml.ml_line_count);
                        let mut pos = CursorPos {
                            lnum,
                            col: c.bufpos.col,
                            coladd: 0,
                        };
                        let mut trow: c_int = 0;
                        let mut tcol: c_int = 0;
                        let mut tcolc: c_int = 0;
                        let mut tcole: c_int = 0;
                        textpos2screenpos(
                            &mut *win, &mut pos, &mut trow, &mut tcol, &mut tcolc, &mut tcole, true,
                        );
                        row += f64::from(trow - 1);
                        col += f64::from(tcol - 1);
                    }
                }
            } else if c.relative == K_FLOAT_RELATIVE_LASTSTATUS {
                row += f64::from(
                    g_st().rows_global - opt_vars().p_ch as c_int - last_stl_height(false),
                );
            } else if c.relative == K_FLOAT_RELATIVE_TABLINE {
                row += f64::from(tabline_height());
            }

            let resort =
                wp.w_grid_alloc.comp_index != 0 && wp.w_grid_alloc.zindex != wp.w_config.zindex;
            let raise = resort && wp.w_grid_alloc.zindex < wp.w_config.zindex;
            wp.w_grid_alloc.zindex = wp.w_config.zindex;
            if resort {
                ui_comp_layers_adjust(wp.w_grid_alloc.comp_index, raise);
            }
            let valid = wp.w_redr_type == 0 || ui_has(K_UIMULTIGRID);
            if !valid && !validate {
                wp.w_pos_changed = true;
                return;
            }

            let east = (c.anchor & K_FLOAT_ANCHOR_EAST) != 0;
            let south = (c.anchor & K_FLOAT_ANCHOR_SOUTH) != 0;

            let mut comp_row = (row as c_int) - if south { wp.w_height_outer } else { 0 };
            let mut comp_col = (col as c_int) - if east { wp.w_width_outer } else { 0 };
            let above_ch = if wp.w_config.zindex < K_ZINDEX_MESSAGES {
                opt_vars().p_ch as c_int
            } else {
                0
            };
            comp_row += (*grid).comp_row;
            comp_col += (*grid).comp_col;
            comp_row = comp_row
                .min(g_st().rows_global - wp.w_height_outer - above_ch)
                .max(0);
            if !c.fixed || east {
                comp_col = comp_col
                    .min(g_st().columns_global - wp.w_width_outer)
                    .max(0);
            }
            wp.w_winrow = comp_row;
            wp.w_wincol = comp_col;

            if !c.hide {
                ui_comp_put_grid(
                    &mut wp.w_grid_alloc,
                    comp_row,
                    comp_col,
                    wp.w_height_outer,
                    wp.w_width_outer,
                    valid,
                    false,
                );
                if ui_has(K_UIMULTIGRID) {
                    let anchor = cstr_as_string(FLOAT_ANCHOR_STR[c.anchor as usize].as_ptr());
                    ui_call_win_float_pos(
                        Integer::from(wp.w_grid_alloc.handle),
                        wp.handle,
                        anchor,
                        Integer::from((*grid).handle),
                        row,
                        col,
                        c.mouse,
                        Integer::from(wp.w_grid_alloc.zindex),
                        wp.w_grid_alloc.comp_index as Integer,
                        Integer::from(wp.w_winrow),
                        Integer::from(wp.w_wincol),
                    );
                }
                ui_check_cursor_grid(wp.w_grid_alloc.handle);
                wp.w_grid_alloc.mouse_enabled = wp.w_config.mouse;
                if !valid {
                    wp.w_grid_alloc.valid = false;
                    redraw_later(Some(&mut *wp), UPD_NOT_VALID);
                }
            } else {
                if ui_has(K_UIMULTIGRID) {
                    ui_call_win_hide(Integer::from(wp.w_grid_alloc.handle));
                }
                ui_comp_remove_grid(&mut wp.w_grid_alloc);
            }
        } else {
            ui_call_win_external_pos(Integer::from(wp.w_grid_alloc.handle), wp.handle);
        }
    }
}

pub fn ui_ext_win_viewport(wp: &mut WinInfoInfo) {
    {
        if (ptr::eq(wp, curwin_ptr()) || ui_has(K_UIMULTIGRID))
            && wp.w_viewport_invalid
            && wp.w_redr_type == 0
        {
            let line_count = wp.buf_mut().b_ml.ml_line_count;
            let cur_topline = wp.w_topline.min(line_count);
            let cur_botline = wp.w_botline.min(line_count);
            let mut delta: i64 = 0;
            let mut last_topline = wp.w_viewport_last_topline;
            let mut last_botline = wp.w_viewport_last_botline;
            let mut last_topfill = wp.w_viewport_last_topfill;
            let mut last_skipcol = i64::from(wp.w_viewport_last_skipcol);
            if last_topline > line_count {
                delta -= i64::from(last_topline - line_count);
                last_topline = line_count;
                last_topfill = 0;
                last_skipcol = i64::from(MAXCOL);
            }
            last_botline = last_botline.min(line_count);
            if cur_topline < last_topline
                || (cur_topline == last_topline && i64::from(wp.w_skipcol) < last_skipcol)
            {
                let mut vcole = last_skipcol;
                let mut lnume = last_topline;
                if last_topline > 0 && cur_botline < last_topline {
                    delta -= i64::from(last_topline - cur_botline);
                    lnume = cur_botline;
                    vcole = 0;
                }
                delta -= win_text_height(
                    wp,
                    cur_topline,
                    i64::from(wp.w_skipcol),
                    &mut lnume,
                    &mut vcole,
                    None,
                    i64::MAX,
                );
            } else if cur_topline > last_topline
                || (cur_topline == last_topline && i64::from(wp.w_skipcol) > last_skipcol)
            {
                let mut vcole = i64::from(wp.w_skipcol);
                let mut lnume = cur_topline;
                if last_botline > 0 && cur_topline > last_botline {
                    delta += i64::from(cur_topline - last_botline);
                    lnume = last_botline;
                    vcole = 0;
                }
                delta += win_text_height(
                    wp,
                    last_topline,
                    last_skipcol,
                    &mut lnume,
                    &mut vcole,
                    None,
                    i64::MAX,
                );
            }
            delta += i64::from(last_topfill);
            delta -= i64::from(wp.w_topfill);
            let mut ev_botline = wp.w_botline;
            if ev_botline == line_count + 1 && wp.w_empty_rows == 0 {
                ev_botline = line_count;
            }
            ui_call_win_viewport(
                Integer::from(wp.w_grid_alloc.handle),
                wp.handle,
                Integer::from(wp.w_topline - 1),
                Integer::from(ev_botline),
                Integer::from(wp.w_cursor.lnum - 1),
                Integer::from(wp.w_cursor.col),
                Integer::from(line_count),
                delta,
            );
            wp.w_viewport_invalid = false;
            wp.w_viewport_last_topline = wp.w_topline;
            wp.w_viewport_last_botline = wp.w_botline;
            wp.w_viewport_last_topfill = wp.w_topfill;
            wp.w_viewport_last_skipcol = wp.w_skipcol;
        }
    }
}

/// If "split_disallowed" is set, or "wp"'s buffer is closing, give an error and return FAIL.
/// Otherwise return OK.
pub fn check_split_disallowed(wp: *const WinInfoInfo) -> bool {
    let mut err = ERROR_INIT;
    let ok = check_split_disallowed_err(wp, &mut err);
    if error_set(&err) {
        emsg(gettext(err.msg_ptr()));
        api_clear_error(&mut err);
    }
    ok
}

/// Like `check_split_disallowed`, but set `err` to the (untranslated) error message on failure and
/// return false. Otherwise return true.
pub fn check_split_disallowed_err(wp: *const WinInfoInfo, err: &mut Error) -> bool {
    unsafe {
        if win_st().split_disallowed > 0 {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"E242: Can't split a window while closing another",
            );
            return false;
        }
        if (*wp).buf_mut().b_locked_split != 0 {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"%s",
                E_CANNOT_SPLIT_WINDOW_WHEN_CLOSING_BUFFER,
            );
            return false;
        }
        true
    }
}

pub fn win_split(size: c_int, mut flags: c_int) -> bool {
    {
        if !check_split_disallowed(curwin_ptr()) {
            return false;
        }

        if may_open_tabpage() {
            return true;
        }

        flags |= g_st().cmdmod.cmod_split;
        if (flags & WSP_TOP) != 0 && (flags & WSP_BOT) != 0 {
            emsg(gettext(
                c"E442: Can't split topleft and botright at the same time",
            ));
            return false;
        }

        if (flags & WSP_HELP) != 0 {
            make_snapshot(SNAP_HELP_IDX as c_int);
        } else {
            clear_snapshot(curtab_mut(), SNAP_HELP_IDX as c_int);
        }

        if (flags & WSP_QUICKFIX) != 0 {
            make_snapshot(SNAP_QUICKFIX_IDX as c_int);
        } else {
            clear_snapshot(curtab_mut(), SNAP_QUICKFIX_IDX as c_int);
        }

        !win_split_ins(size, flags, ptr::null_mut(), 0, ptr::null_mut()).is_null()
    }
}

pub fn win_split_ins(
    size: c_int,
    flags: c_int,
    new_wp: *mut WinInfoInfo,
    dir: c_int,
    to_flatten: *mut Frame,
) -> *mut WinInfoInfo {
    unsafe {
        let mut wp = new_wp;

        if !new_wp.is_null() && is_ctx_win(new_wp) {
            return ptr::null_mut();
        }

        if new_wp.is_null() {
            trigger_winnewpre();
        }

        let oldwin = if (flags & WSP_TOP) != 0 {
            g_st().firstwin
        } else if (flags & WSP_BOT) != 0 || curwin_mut().w_floating {
            lastwin_nofloating(ptr::null_mut())
        } else {
            curwin_ptr()
        };

        let mut need_status: c_int = 0;
        let mut new_size = size;
        let vertical = (flags & WSP_VERT) != 0;
        let toplevel = (flags & (WSP_TOP | WSP_BOT)) != 0;

        if one_window(firstwin_mut(), ptr::null_mut())
            && opt_vars().p_ls == 1
            && (*oldwin).w_status_height == 0
        {
            if OptInt::from((*oldwin).w_height) <= opt_vars().p_wmh {
                emsg(gettext(E_NOROOM));
                return ptr::null_mut();
            }
            need_status = STATUS_HEIGHT;
            win_float_anchor_laststatus();
        }

        let mut do_equal = false;
        let mut oldwin_height: c_int = 0;
        let layout = if vertical { FR_ROW } else { FR_COL };
        let mut did_set_fraction = false;

        if vertical {
            let wmw1 = if opt_vars().p_wmw == 0 {
                1
            } else {
                opt_vars().p_wmw as c_int
            };
            let mut needed = wmw1 + 1;
            if (flags & WSP_ROOM) != 0 {
                needed += opt_vars().p_wiw as c_int - wmw1;
            }
            let mut minwidth: c_int;
            let available: c_int;
            if toplevel {
                minwidth = frame_minwidth(&mut *g_st().topframe, nowin());
                available = (*g_st().topframe).fr_width;
                needed += minwidth;
            } else if opt_vars().p_ea != 0 {
                minwidth = frame_minwidth(&mut *(*oldwin).w_frame, nowin());
                let mut prevfrp = (*oldwin).w_frame;
                let mut frp = (*(*oldwin).w_frame).fr_parent;
                while !frp.is_null() {
                    if (*frp).fr_layout == FR_ROW {
                        for frp2 in frame_iter_ptr((*frp).fr_child) {
                            if !ptr::eq(frp2, prevfrp) {
                                minwidth += frame_minwidth(&mut *frp2, nowin());
                            }
                        }
                    }
                    prevfrp = frp;
                    frp = (*frp).fr_parent;
                }
                available = (*g_st().topframe).fr_width;
                needed += minwidth;
            } else {
                minwidth = frame_minwidth(&mut *(*oldwin).w_frame, nowin());
                available = (*(*oldwin).w_frame).fr_width;
                needed += minwidth;
            }
            if available < needed {
                emsg(gettext(E_NOROOM));
                return ptr::null_mut();
            }
            if new_size == 0 {
                new_size = (*oldwin).w_width / 2;
            }
            new_size = new_size.min(available - minwidth - 1).max(wmw1);

            if OptInt::from((*oldwin).w_width - new_size - 1) < opt_vars().p_wmw {
                do_equal = true;
            }

            if (*oldwin).w_onebuf_opt.wo_wfw != 0 {
                win_setwidth_win((*oldwin).w_width + new_size + 1, &mut *oldwin, true);
            }

            if !do_equal
                && opt_vars().p_ea != 0
                && size == 0
                && crate::memory::cstr_first_byte(opt_vars().p_ead) != b'v'
                && !(*(*oldwin).w_frame).fr_parent.is_null()
            {
                for frp in frame_iter_mut((*(*(*oldwin).w_frame).fr_parent).fr_child) {
                    if !ptr::eq(frp.fr_win, oldwin)
                        && !frp.fr_win.is_null()
                        && ((*frp.fr_win).w_width > new_size
                            || (*frp.fr_win).w_width > ((*oldwin).w_width - new_size - 1))
                    {
                        do_equal = true;
                        break;
                    }
                }
            }
        } else {
            let wmh1 = (opt_vars().p_wmh as c_int).max(1) + (*oldwin).w_winbar_height;
            let mut needed = wmh1 + STATUS_HEIGHT;
            if (flags & WSP_ROOM) != 0 {
                needed += opt_vars().p_wh as c_int - wmh1 + (*oldwin).w_winbar_height;
            }
            if opt_vars().p_ch < 1 {
                needed += 1;
            }
            let mut minheight: c_int;
            let available: c_int;
            if toplevel {
                minheight = frame_minheight(&mut *g_st().topframe, nowin()) + need_status;
                available = (*g_st().topframe).fr_height;
                needed += minheight;
            } else if opt_vars().p_ea != 0 {
                minheight = frame_minheight(&mut *(*oldwin).w_frame, nowin()) + need_status;
                let mut prevfrp = (*oldwin).w_frame;
                let mut frp = (*(*oldwin).w_frame).fr_parent;
                while !frp.is_null() {
                    if (*frp).fr_layout == FR_COL {
                        for frp2 in frame_iter_ptr((*frp).fr_child) {
                            if !ptr::eq(frp2, prevfrp) {
                                minheight += frame_minheight(&mut *frp2, nowin());
                            }
                        }
                    }
                    prevfrp = frp;
                    frp = (*frp).fr_parent;
                }
                available = (*g_st().topframe).fr_height;
                needed += minheight;
            } else {
                minheight = frame_minheight(&mut *(*oldwin).w_frame, nowin()) + need_status;
                available = (*(*oldwin).w_frame).fr_height;
                needed += minheight;
            }
            if available < needed {
                emsg(gettext(E_NOROOM));
                return ptr::null_mut();
            }
            oldwin_height = (*oldwin).w_height;
            if need_status != 0 {
                (*oldwin).w_status_height = STATUS_HEIGHT;
                oldwin_height -= STATUS_HEIGHT;
            }
            if new_size == 0 {
                new_size = oldwin_height / 2;
            }

            new_size = new_size
                .min(available - minheight - STATUS_HEIGHT)
                .max(wmh1);

            if OptInt::from(oldwin_height - new_size - STATUS_HEIGHT) < opt_vars().p_wmh {
                do_equal = true;
            }

            if (*oldwin).w_onebuf_opt.wo_wfh != 0 {
                set_fraction(&mut *oldwin);
                did_set_fraction = true;

                win_setheight_win(
                    (*oldwin).w_height + new_size + STATUS_HEIGHT,
                    &mut *oldwin,
                    true,
                );
                oldwin_height = (*oldwin).w_height;
            }

            if !do_equal
                && opt_vars().p_ea != 0
                && size == 0
                && crate::memory::cstr_first_byte(opt_vars().p_ead) != b'h'
                && !(*(*oldwin).w_frame).fr_parent.is_null()
            {
                for frp in frame_iter_mut((*(*(*oldwin).w_frame).fr_parent).fr_child) {
                    if !ptr::eq(frp.fr_win, oldwin)
                        && !frp.fr_win.is_null()
                        && ((*frp.fr_win).w_height > new_size
                            || (*frp.fr_win).w_height > oldwin_height - new_size - STATUS_HEIGHT)
                    {
                        do_equal = true;
                        break;
                    }
                }
            }
        }

        if (flags & WSP_TOP) == 0
            && ((flags & WSP_BOT) != 0
                || (flags & WSP_BELOW) != 0
                || ((flags & WSP_ABOVE) == 0
                    && if vertical {
                        opt_vars().p_spr != 0
                    } else {
                        opt_vars().p_sb != 0
                    }))
        {
            if new_wp.is_null() {
                wp = win_alloc(oldwin, false);
            } else {
                win_append(oldwin, wp, ptr::null_mut());
            }
        } else if new_wp.is_null() {
            wp = win_alloc((*oldwin).w_prev, false);
        } else {
            win_append((*oldwin).w_prev, wp, ptr::null_mut());
        }

        if new_wp.is_null() {
            if wp.is_null() {
                return ptr::null_mut();
            }

            new_frame(&mut *wp);
            win_init(&mut *wp, curwin_mut(), flags);
        } else if (*wp).w_floating {
            ui_comp_remove_grid(&mut (*wp).w_grid_alloc);
            if ui_has(K_UIMULTIGRID) {
                (*wp).w_pos_changed = true;
            } else {
                ui_call_win_hide(Integer::from((*wp).w_grid_alloc.handle));
                win_free_grid(&mut *wp, true);
            }

            if (*wp).w_config.external {
                for tp in for_all_tabpages_ptr() {
                    if !ptr::eq(tp, g_st().curtab) && ptr::eq((*tp).tp_curwin, wp) {
                        (*tp).tp_curwin = (*tp).tp_firstwin;
                    }
                }
            }

            (*wp).w_floating = false;
            new_frame(&mut *wp);

            clear_float_config(&mut (*wp).w_config, true);
            (*wp).w_border_adj = [0; 4];
        }

        if !to_flatten.is_null() {
            frame_flatten(to_flatten);
        }

        let before: bool;
        let mut curfrp: *mut Frame;

        if toplevel {
            if ((*g_st().topframe).fr_layout == FR_COL && !vertical)
                || ((*g_st().topframe).fr_layout == FR_ROW && vertical)
            {
                curfrp = (*g_st().topframe).fr_child;
                if (flags & WSP_BOT) != 0 {
                    while !(*curfrp).fr_next.is_null() {
                        curfrp = (*curfrp).fr_next;
                    }
                }
            } else {
                curfrp = g_st().topframe;
            }
            before = (flags & WSP_TOP) != 0;
        } else {
            curfrp = (*oldwin).w_frame;
            if (flags & WSP_BELOW) != 0 {
                before = false;
            } else if (flags & WSP_ABOVE) != 0 {
                before = true;
            } else if vertical {
                before = opt_vars().p_spr == 0;
            } else {
                before = opt_vars().p_sb == 0;
            }
        }
        if (*curfrp).fr_parent.is_null() || (*(*curfrp).fr_parent).fr_layout != layout {
            let mut frp = Box::into_raw(Box::new(Frame {
                fr_parent: curfrp,
                fr_next: ptr::null_mut(),
                fr_prev: ptr::null_mut(),
                ..*curfrp
            }));
            (*curfrp).fr_layout = layout;
            (*curfrp).fr_child = frp;
            (*curfrp).fr_win = ptr::null_mut();
            curfrp = frp;
            if !(*frp).fr_win.is_null() {
                (*oldwin).w_frame = frp;
            } else {
                frp = (*frp).fr_child;
                while !frp.is_null() {
                    (*frp).fr_parent = curfrp;
                    frp = (*frp).fr_next;
                }
            }
        }

        let frp = if new_wp.is_null() {
            (*wp).w_frame
        } else {
            (*new_wp).w_frame
        };
        (*frp).fr_parent = (*curfrp).fr_parent;

        if before {
            frame_insert(&mut *curfrp, &mut *frp);
        } else {
            frame_append(&mut *curfrp, &mut *frp);
        }

        if !did_set_fraction {
            set_fraction(&mut *oldwin);
        }
        (*wp).w_fraction = (*oldwin).w_fraction;

        if vertical {
            (*wp).w_onebuf_opt.wo_scr = curwin_mut().w_onebuf_opt.wo_scr;

            if need_status != 0 {
                win_new_height(&mut *oldwin, (*oldwin).w_height - 1);
                (*oldwin).w_status_height = need_status;
            }
            if toplevel {
                (*wp).w_winrow = tabline_height();
                let ls_h = c_int::from(opt_vars().p_ls == 1 || opt_vars().p_ls == 2);
                win_new_height(&mut *wp, (*curfrp).fr_height - ls_h);
                (*wp).w_status_height = ls_h;
                (*wp).w_hsep_height = 0;
            } else {
                (*wp).w_winrow = (*oldwin).w_winrow;
                win_new_height(&mut *wp, (*oldwin).w_height);
                (*wp).w_status_height = (*oldwin).w_status_height;
                (*wp).w_hsep_height = (*oldwin).w_hsep_height;
            }
            (*frp).fr_height = (*curfrp).fr_height;

            win_new_width(&mut *wp, new_size);
            if before {
                (*wp).w_vsep_width = 1;
            } else {
                (*wp).w_vsep_width = (*oldwin).w_vsep_width;
                (*oldwin).w_vsep_width = 1;
            }
            if toplevel {
                if (flags & WSP_BOT) != 0 {
                    frame_set_vsep(curfrp, true);
                }
                frame_new_width(
                    &mut *curfrp,
                    (*curfrp).fr_width - (new_size + c_int::from((flags & WSP_TOP) != 0)),
                    (flags & WSP_TOP) != 0,
                    false,
                );
            } else {
                win_new_width(&mut *oldwin, (*oldwin).w_width - (new_size + 1));
            }
            if before {
                (*wp).w_wincol = (*oldwin).w_wincol;
                (*oldwin).w_wincol += new_size + 1;
            } else {
                (*wp).w_wincol = (*oldwin).w_wincol + (*oldwin).w_width + 1;
            }
            frame_fix_width(&mut *oldwin);
            frame_fix_width(&mut *wp);
        } else {
            let is_stl_global = global_stl_height() > 0;
            if toplevel {
                (*wp).w_wincol = 0;
                win_new_width(&mut *wp, g_st().columns_global);
                (*wp).w_vsep_width = 0;
            } else {
                (*wp).w_wincol = (*oldwin).w_wincol;
                win_new_width(&mut *wp, (*oldwin).w_width);
                (*wp).w_vsep_width = (*oldwin).w_vsep_width;
            }
            (*frp).fr_width = (*curfrp).fr_width;

            win_new_height(&mut *wp, new_size);
            let old_status_height = (*oldwin).w_status_height;
            if before {
                (*wp).w_hsep_height = c_int::from(is_stl_global);
            } else {
                (*wp).w_hsep_height = (*oldwin).w_hsep_height;
                (*oldwin).w_hsep_height = c_int::from(is_stl_global);
            }
            if toplevel {
                let mut new_fr_height = (*curfrp).fr_height - new_size;
                if is_stl_global {
                    if (flags & WSP_BOT) != 0 {
                        frame_add_hsep(curfrp);
                    } else {
                        new_fr_height -= 1;
                    }
                } else {
                    if !((flags & WSP_BOT) != 0 && opt_vars().p_ls == 0) {
                        new_fr_height -= STATUS_HEIGHT;
                    }
                    if (flags & WSP_BOT) != 0 {
                        frame_add_statusline(curfrp);
                    }
                }
                frame_new_height(curfrp, new_fr_height, (flags & WSP_TOP) != 0, false, false);
            } else {
                win_new_height(&mut *oldwin, oldwin_height - (new_size + STATUS_HEIGHT));
            }

            if before {
                (*wp).w_winrow = (*oldwin).w_winrow;
                if is_stl_global {
                    (*wp).w_status_height = 0;
                    (*oldwin).w_winrow += (*wp).w_height + 1;
                } else {
                    (*wp).w_status_height = STATUS_HEIGHT;
                    (*oldwin).w_winrow += (*wp).w_height + STATUS_HEIGHT;
                }
            } else if is_stl_global {
                (*wp).w_winrow = (*oldwin).w_winrow + (*oldwin).w_height + 1;
                (*wp).w_status_height = 0;
            } else {
                (*wp).w_winrow = (*oldwin).w_winrow + (*oldwin).w_height + STATUS_HEIGHT;
                (*wp).w_status_height = old_status_height;
                if (flags & WSP_BOT) == 0 {
                    (*oldwin).w_status_height = STATUS_HEIGHT;
                }
            }
            frame_fix_height(&mut *wp);
            frame_fix_height(&mut *oldwin);
        }

        if toplevel {
            win_comp_pos();
        }

        redraw_later(wp.as_mut(), UPD_NOT_VALID);
        redraw_later(oldwin.as_mut(), UPD_NOT_VALID);
        status_redraw_all();

        if need_status != 0 {
            g_st().msg_row = g_st().rows_global - 1;
            g_st().msg_col = g_st().sc_col;
            msg_clr_eos_force();
            comp_col();
            g_st().msg_row = g_st().rows_global - 1;
            g_st().msg_col = 0;
        }

        if do_equal || dir != 0 {
            win_equal(
                wp,
                true,
                if vertical {
                    if dir == b'v' as c_int {
                        b'b' as c_int
                    } else {
                        b'h' as c_int
                    }
                } else if dir == b'h' as c_int {
                    b'b' as c_int
                } else {
                    b'v' as c_int
                },
            );
        } else if !is_ctx_win(wp) {
            win_fix_scroll(false);
        }

        let i: c_int;
        if (flags & WSP_VERT) != 0 {
            i = opt_vars().p_wiw as c_int;
            if size != 0 {
                opt_vars().p_wiw = OptInt::from(size);
            }
        } else {
            i = opt_vars().p_wh as c_int;
            if size != 0 {
                opt_vars().p_wh = OptInt::from(size);
            }
        }

        if (flags & WSP_NOENTER) == 0 {
            win_enter_ext(
                wp,
                (if new_wp.is_null() {
                    WEE_TRIGGER_NEW_AUTOCMDS
                } else {
                    0
                }) | WEE_TRIGGER_ENTER_AUTOCMDS
                    | WEE_TRIGGER_LEAVE_AUTOCMDS,
            );
        }
        if vertical {
            opt_vars().p_wiw = OptInt::from(i);
        } else {
            opt_vars().p_wh = OptInt::from(i);
        }

        if win_valid(oldwin) {
            (*oldwin).w_pos_changed = true;
        }

        wp
    }
}

pub fn win_init(newp: &mut WinInfoInfo, oldp: &mut WinInfoInfo, flags: c_int) {
    {
        newp.w_buffer = oldp.w_buffer;
        newp.w_s_owned = None;
        oldp.buf_mut().b_nwindows += 1;
        newp.w_cursor = oldp.w_cursor;
        newp.w_valid = 0;
        newp.w_curswant = oldp.w_curswant;
        newp.w_set_curswant = oldp.w_set_curswant;
        newp.w_topline = oldp.w_topline;
        newp.w_topfill = oldp.w_topfill;
        newp.w_leftcol = oldp.w_leftcol;
        newp.w_pcmark = oldp.w_pcmark;
        newp.w_prev_pcmark = oldp.w_prev_pcmark;
        newp.w_alt_fnum = oldp.w_alt_fnum;
        newp.w_wrow = oldp.w_wrow;
        newp.w_fraction = oldp.w_fraction;
        newp.w_prev_fraction_row = oldp.w_prev_fraction_row;
        copy_jumplist(oldp, newp);
        if (flags & WSP_NEWLOC) != 0 {
            newp.w_llist = ptr::null_mut();
            newp.w_llist_ref = ptr::null_mut();
        } else {
            copy_loclist_stack(oldp, newp);
        }
        newp.w_localdir = oldp.w_localdir.clone();
        newp.w_prevdir = oldp.w_prevdir.clone();

        if crate::memory::cstr_first_byte(opt_vars().p_spk) != b'c' {
            if crate::memory::cstr_first_byte(opt_vars().p_spk) == b't' {
                newp.w_skipcol = oldp.w_skipcol;
            }
            newp.w_botline = oldp.w_botline;
            newp.w_prev_height = oldp.w_height;
            newp.w_prev_winrow = oldp.w_winrow;
        }

        for i in 0..oldp.w_tagstacklen as usize {
            newp.w_tagstack[i] = oldp.w_tagstack[i].clone();
        }
        newp.w_tagstackidx = oldp.w_tagstackidx;
        newp.w_tagstacklen = oldp.w_tagstacklen;

        newp.w_changelistidx = oldp.w_changelistidx;

        copy_folding_state(oldp, newp);

        win_init_some(newp, oldp);

        newp.w_winbar_height = oldp.w_winbar_height;
    }
}

fn win_init_some(newp: &mut WinInfoInfo, oldp: &mut WinInfoInfo) {
    newp.w_alist = oldp.w_alist.clone();
    newp.w_arg_idx = oldp.w_arg_idx;

    win_copy_options(oldp, newp);
}

pub fn win_valid(win: *const WinInfoInfo) -> bool {
    tabpage_win_valid(g_st().curtab, win)
}

pub fn tabpage_win_valid(tp: *const TabPageInfo, win: *const WinInfoInfo) -> bool {
    if win.is_null() {
        return false;
    }

    for wp in for_all_windows_in_tab_ptr(unsafe { &*tp }) {
        if ptr::eq(wp, win) {
            return true;
        }
    }
    false
}

pub fn win_find_by_handle(handle: ObjHandle) -> *mut WinInfoInfo {
    unsafe {
        for wp in for_all_windows_ptr() {
            if (*wp).handle == handle {
                return wp;
            }
        }
        ptr::null_mut()
    }
}

pub fn win_valid_any_tab(win: *const WinInfoInfo) -> bool {
    if win.is_null() {
        return false;
    }

    for tp in for_all_tabpages_mut() {
        for wp in for_all_windows_in_tab_ptr(tp) {
            if ptr::eq(wp, win) {
                return true;
            }
        }
    }
    false
}

pub fn win_count() -> c_int {
    {
        let mut count: c_int = 0;
        for _wp in for_all_windows_mut() {
            count += 1;
        }
        count
    }
}

pub fn make_windows(mut count: c_int, vertical: bool) -> c_int {
    unsafe {
        let mut maxcount = if vertical {
            ((OptInt::from(curwin_mut().w_width + curwin_mut().w_vsep_width)
                - (opt_vars().p_wiw - opt_vars().p_wmw)) as c_int)
                / (opt_vars().p_wmw as c_int + 1)
        } else {
            ((OptInt::from(
                curwin_mut().w_height + curwin_mut().w_hsep_height + curwin_mut().w_status_height,
            ) - (opt_vars().p_wh - opt_vars().p_wmh)) as c_int)
                / (opt_vars().p_wmh as c_int + STATUS_HEIGHT + global_winbar_height())
        };

        maxcount = maxcount.max(2);
        count = count.min(maxcount);

        if count > 1 {
            last_status(true);
        }

        block_autocmds();

        let mut todo = count - 1;
        while todo > 0 {
            if vertical {
                if !win_split(
                    curwin_mut().w_width - (curwin_mut().w_width - todo) / (todo + 1) - 1,
                    WSP_VERT | WSP_ABOVE,
                ) {
                    break;
                }
            } else if !win_split(
                curwin_mut().w_height
                    - (curwin_mut().w_height - todo * STATUS_HEIGHT) / (todo + 1)
                    - STATUS_HEIGHT,
                WSP_ABOVE,
            ) {
                break;
            }
            todo -= 1;
        }

        unblock_autocmds();

        count - todo
    }
}

fn win_exchange(mut prenum: c_int) {
    unsafe {
        if curwin_mut().w_floating {
            emsg(E_FLOATEXCHANGE);
            return;
        }

        if one_window(curwin_mut(), ptr::null_mut()) {
            beep_flush();
            return;
        }
        if text_or_buf_locked() {
            beep_flush();
            return;
        }

        let mut frp: *mut Frame;
        if prenum != 0 {
            frp = (*(*curwin_mut().w_frame).fr_parent).fr_child;
            while !frp.is_null() && {
                prenum -= 1;
                prenum > 0
            } {
                frp = (*frp).fr_next;
            }
        } else if !(*curwin_mut().w_frame).fr_next.is_null() {
            frp = (*curwin_mut().w_frame).fr_next;
        } else {
            frp = (*curwin_mut().w_frame).fr_prev;
        }

        if frp.is_null() || (*frp).fr_win.is_null() || ptr::eq((*frp).fr_win, curwin_ptr()) {
            return;
        }
        let wp = (*frp).fr_win;

        let wp2 = curwin_mut().w_prev;
        let frp2 = (*curwin_mut().w_frame).fr_prev;
        if !ptr::eq((*wp).w_prev, curwin_ptr()) {
            win_remove(curwin_mut(), ptr::null_mut());
            frame_remove(&mut *curwin_mut().w_frame);
            win_append((*wp).w_prev, curwin_ptr(), ptr::null_mut());
            frame_insert(&mut *frp, &mut *curwin_mut().w_frame);
        }
        if !ptr::eq(wp, wp2) {
            win_remove(&mut *wp, ptr::null_mut());
            frame_remove(&mut *(*wp).w_frame);
            win_append(wp2, wp, ptr::null_mut());
            if frp2.is_null() {
                frame_insert(
                    &mut *(*(*(*wp).w_frame).fr_parent).fr_child,
                    &mut *(*wp).w_frame,
                );
            } else {
                frame_append(&mut *frp2, &mut *(*wp).w_frame);
            }
        }
        std::mem::swap(
            &mut curwin_mut().w_status_height,
            &mut (*wp).w_status_height,
        );
        std::mem::swap(&mut curwin_mut().w_vsep_width, &mut (*wp).w_vsep_width);
        std::mem::swap(&mut curwin_mut().w_hsep_height, &mut (*wp).w_hsep_height);

        frame_fix_height(curwin_mut());
        frame_fix_height(&mut *wp);
        frame_fix_width(curwin_mut());
        frame_fix_width(&mut *wp);

        win_comp_pos();

        if !ptr::eq((*wp).w_buffer, g_st().curbuf) {
            reset_visual_and_resel();
        } else if g_st().visual_global.active {
            (*wp).w_cursor = curwin_mut().w_cursor;
        }

        win_enter(wp, true);
        redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
    }
}

fn win_rotate(upwards: bool, mut count: c_int) {
    unsafe {
        if curwin_mut().w_floating {
            emsg(E_FLOATEXCHANGE);
            return;
        }

        if count <= 0 || one_window(curwin_mut(), ptr::null_mut()) {
            beep_flush();
            return;
        }

        let mut frp = (*(*curwin_mut().w_frame).fr_parent).fr_child;
        while !frp.is_null() {
            if (*frp).fr_win.is_null() {
                emsg(gettext(c"E443: Cannot rotate when another window is split"));
                return;
            }
            frp = (*frp).fr_next;
        }

        let mut wp1: *mut WinInfoInfo = ptr::null_mut();
        let mut wp2: *mut WinInfoInfo = ptr::null_mut();

        while count > 0 {
            count -= 1;
            if upwards {
                frp = (*(*curwin_mut().w_frame).fr_parent).fr_child;
                debug_assert!(!frp.is_null());
                wp1 = (*frp).fr_win;
                win_remove(&mut *wp1, ptr::null_mut());
                frame_remove(&mut *frp);
                debug_assert!(!(*(*frp).fr_parent).fr_child.is_null());

                while !(*frp).fr_next.is_null() {
                    frp = (*frp).fr_next;
                }
                win_append((*frp).fr_win, wp1, ptr::null_mut());
                frame_append(&mut *frp, &mut *(*wp1).w_frame);

                wp2 = (*frp).fr_win;
            } else {
                frp = curwin_mut().w_frame;
                while !(*frp).fr_next.is_null() {
                    frp = (*frp).fr_next;
                }
                wp1 = (*frp).fr_win;
                wp2 = (*wp1).w_prev;
                win_remove(&mut *wp1, ptr::null_mut());
                frame_remove(&mut *frp);
                debug_assert!(!(*(*frp).fr_parent).fr_child.is_null());

                win_append(
                    (*(*(*(*frp).fr_parent).fr_child).fr_win).w_prev,
                    wp1,
                    ptr::null_mut(),
                );
                frame_insert(&mut *(*(*frp).fr_parent).fr_child, &mut *frp);
            }

            std::mem::swap(&mut (*wp2).w_status_height, &mut (*wp1).w_status_height);
            std::mem::swap(&mut (*wp2).w_hsep_height, &mut (*wp1).w_hsep_height);
            frame_fix_height(&mut *wp1);
            frame_fix_height(&mut *wp2);
            std::mem::swap(&mut (*wp2).w_vsep_width, &mut (*wp1).w_vsep_width);
            frame_fix_width(&mut *wp1);
            frame_fix_width(&mut *wp2);

            win_comp_pos();
        }

        (*wp1).w_pos_changed = true;
        (*wp2).w_pos_changed = true;

        redraw_all_later(UPD_NOT_VALID);
    }
}

pub fn win_splitmove(wp: &mut WinInfoInfo, size: c_int, flags: c_int) -> bool {
    unsafe {
        let mut dir: c_int = 0;
        let height = wp.w_height;

        if one_window(wp, ptr::null_mut()) {
            return true;
        }
        if is_ctx_win(&raw mut *wp) || !check_split_disallowed(&raw mut *wp) {
            return false;
        }

        let mut unflat_altfr: *mut Frame = ptr::null_mut();
        if wp.w_floating {
            win_remove(wp, ptr::null_mut());
        } else {
            winframe_remove(wp, &mut dir, ptr::null_mut(), &raw mut unflat_altfr);
            debug_assert!(!unflat_altfr.is_null());
            win_remove(wp, ptr::null_mut());
            last_status(false);
            win_comp_pos();
        }

        if win_split_ins(size, flags, (&raw mut *wp), dir, unflat_altfr).is_null() {
            if !wp.w_floating {
                debug_assert!(!unflat_altfr.is_null());
                winframe_restore(wp, dir, &mut *unflat_altfr);
            }
            win_append(wp.w_prev, (&raw mut *wp), ptr::null_mut());
            return false;
        }

        if size == 0 && (flags & WSP_VERT) == 0 && win_valid(&raw mut *wp) && !wp.w_floating {
            win_setheight_win(height, wp, true);
            if opt_vars().p_ea != 0 {
                win_equal(
                    curwin_ptr(),
                    ptr::eq(curwin_ptr(), (&raw mut *wp)),
                    b'v' as c_int,
                );
            }
        }

        true
    }
}

pub fn win_move_after(win1: &mut WinInfoInfo, win2: &mut WinInfoInfo) {
    unsafe {
        if ptr::eq(win1, win2) {
            return;
        }

        if !ptr::eq(win2.w_next, win1) {
            if !ptr::eq((*win1.w_frame).fr_parent, (*win2.w_frame).fr_parent) {
                iemsg(c"INTERNAL: trying to move a window into another frame");
                return;
            }

            if ptr::eq(win1, g_st().lastwin) {
                std::mem::swap(
                    &mut (*win1.w_prev).w_status_height,
                    &mut win1.w_status_height,
                );
                std::mem::swap(&mut (*win1.w_prev).w_hsep_height, &mut win1.w_hsep_height);

                if (*win1.w_prev).w_vsep_width == 1 {
                    (*win1.w_prev).w_vsep_width = 0;
                    (*(*win1.w_prev).w_frame).fr_width -= 1;
                    win1.w_vsep_width = 1;
                    (*win1.w_frame).fr_width += 1;
                }
            } else if ptr::eq(win2, g_st().lastwin) {
                std::mem::swap(&mut win1.w_status_height, &mut win2.w_status_height);
                std::mem::swap(&mut win1.w_hsep_height, &mut win2.w_hsep_height);

                if win1.w_vsep_width == 1 {
                    win2.w_vsep_width = 1;
                    (*win2.w_frame).fr_width += 1;
                    win1.w_vsep_width = 0;
                    (*win1.w_frame).fr_width -= 1;
                }
            }
            win_remove(&mut *win1, ptr::null_mut());
            frame_remove(&mut *win1.w_frame);
            win_append(&raw mut *win2, &raw mut *win1, ptr::null_mut());
            frame_append(&mut *win2.w_frame, &mut *win1.w_frame);

            win_comp_pos();
            redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
        }
        win1.w_pos_changed = true;
        win2.w_pos_changed = true;

        win_enter(&raw mut *win1, false);
    }
}

fn get_maximum_wincount(fr: &mut Frame, mut height: c_int) -> c_int {
    {
        if fr.fr_layout != FR_COL {
            return height
                / (opt_vars().p_wmh as c_int
                    + STATUS_HEIGHT
                    + frame2win(&raw mut *fr).w_winbar_height);
        } else if global_winbar_height() != 0 {
            return height / (opt_vars().p_wmh as c_int + STATUS_HEIGHT + 1);
        }

        let mut total_wincount: c_int = 0;
        for frp in frame_iter_ptr(fr.fr_child) {
            let wp = frame2win(frp);
            if OptInt::from(height)
                < (opt_vars().p_wmh + OptInt::from(STATUS_HEIGHT + wp.w_winbar_height))
            {
                break;
            }
            height -= opt_vars().p_wmh as c_int + STATUS_HEIGHT + wp.w_winbar_height;
            total_wincount += 1;
        }

        total_wincount += height / (opt_vars().p_wmh as c_int + STATUS_HEIGHT);
        total_wincount
    }
}

pub fn win_equal(next_curwin: *mut WinInfoInfo, current: bool, mut dir: c_int) {
    unsafe {
        if dir == 0 {
            dir = c_int::from(crate::memory::cstr_first_byte(opt_vars().p_ead));
        }
        win_equal_rec(
            if next_curwin.is_null() {
                curwin_ptr()
            } else {
                next_curwin
            },
            current,
            &mut *g_st().topframe,
            dir,
            0,
            tabline_height(),
            g_st().columns_global,
            (*g_st().topframe).fr_height,
        );
        if !is_ctx_win(next_curwin) {
            win_fix_scroll(true);
        }
    }
}

fn win_equal_rec(
    next_curwin: *mut WinInfoInfo,
    current: bool,
    topfr: &mut Frame,
    dir: c_int,
    mut col: c_int,
    mut row: c_int,
    mut width: c_int,
    mut height: c_int,
) {
    unsafe {
        let mut extra_sep: c_int = 0;
        let mut totwincount: c_int = 0;
        let mut next_curwin_size: c_int = 0;
        let mut room: c_int = 0;
        let mut has_next_curwin = false;

        if topfr.fr_layout == FR_LEAF {
            if topfr.fr_height != height
                || (*topfr.fr_win).w_winrow != row
                || topfr.fr_width != width
                || (*topfr.fr_win).w_wincol != col
            {
                (*topfr.fr_win).w_winrow = row;
                frame_new_height(&raw mut *topfr, height, false, false, false);
                (*topfr.fr_win).w_wincol = col;
                frame_new_width(topfr, width, false, false);
                redraw_all_later(UPD_NOT_VALID);
            }
        } else if topfr.fr_layout == FR_ROW {
            topfr.fr_width = width;
            topfr.fr_height = height;

            if dir != b'v' as c_int {
                let mut n = frame_minwidth(topfr, nowin());
                if col + width == g_st().columns_global {
                    extra_sep = 1;
                } else {
                    extra_sep = 0;
                }
                totwincount = (n + extra_sep) / (opt_vars().p_wmw as c_int + 1);
                has_next_curwin = frame_has_win(topfr, next_curwin);

                let m = frame_minwidth(topfr, next_curwin);
                room = width - m;
                if room < 0 {
                    next_curwin_size = opt_vars().p_wiw as c_int + room;
                    room = 0;
                } else {
                    next_curwin_size = -1;
                    for fr in frame_iter_ptr(topfr.fr_child) {
                        if frame_fixed_width(fr) {
                            n = frame_minwidth(&mut *fr, nowin());
                            let mut new_size = (*fr).fr_width;
                            if frame_has_win(&*fr, next_curwin) {
                                room += opt_vars().p_wiw as c_int - opt_vars().p_wmw as c_int;
                                next_curwin_size = 0;
                                new_size = new_size.max(opt_vars().p_wiw as c_int);
                            } else {
                                totwincount -= (n + if (*fr).fr_next.is_null() {
                                    extra_sep
                                } else {
                                    0
                                }) / (opt_vars().p_wmw as c_int + 1);
                            }
                            room -= new_size - n;
                            if room < 0 {
                                new_size += room;
                                room = 0;
                            }
                            (*fr).fr_newwidth = new_size;
                        }
                    }
                    if next_curwin_size == -1 {
                        if !has_next_curwin {
                            next_curwin_size = 0;
                        } else if totwincount > 1
                            && OptInt::from((room + (totwincount - 2)) / (totwincount - 1))
                                > opt_vars().p_wiw
                        {
                            next_curwin_size = ((OptInt::from(room)
                                + opt_vars().p_wiw
                                + OptInt::from(totwincount - 1) * opt_vars().p_wmw
                                + OptInt::from(totwincount - 1))
                                as c_int)
                                / totwincount;
                            room -= next_curwin_size - opt_vars().p_wiw as c_int;
                        } else {
                            next_curwin_size = opt_vars().p_wiw as c_int;
                        }
                    }
                }

                if has_next_curwin {
                    totwincount -= 1;
                }
            }

            for fr in frame_iter_ptr(topfr.fr_child) {
                let mut wincount: c_int = 1;
                let new_size: c_int;
                if (*fr).fr_next.is_null() {
                    new_size = width;
                } else if dir == b'v' as c_int {
                    new_size = (*fr).fr_width;
                } else if frame_fixed_width(fr) {
                    new_size = (*fr).fr_newwidth;
                    wincount = 0;
                } else {
                    let n = frame_minwidth(&mut *fr, nowin());
                    wincount = (n + if (*fr).fr_next.is_null() {
                        extra_sep
                    } else {
                        0
                    }) / (opt_vars().p_wmw as c_int + 1);
                    let m = frame_minwidth(&mut *fr, next_curwin);
                    let hnc = has_next_curwin && frame_has_win(&*fr, next_curwin);
                    if hnc {
                        wincount -= 1;
                    }
                    let mut ns = if totwincount == 0 {
                        room
                    } else {
                        (wincount * room + (totwincount / 2)) / totwincount
                    };
                    if hnc {
                        next_curwin_size -= opt_vars().p_wiw as c_int - (m - n);
                        next_curwin_size = next_curwin_size.max(0);
                        ns += next_curwin_size;
                        room -= ns - next_curwin_size;
                    } else {
                        room -= ns;
                    }
                    new_size = ns + n;
                }

                if !current
                    || dir != b'v' as c_int
                    || !topfr.fr_parent.is_null()
                    || new_size != (*fr).fr_width
                    || frame_has_win(&*fr, next_curwin)
                {
                    win_equal_rec(
                        next_curwin,
                        current,
                        &mut *fr,
                        dir,
                        col,
                        row,
                        new_size,
                        height,
                    );
                }
                col += new_size;
                width -= new_size;
                totwincount -= wincount;
            }
        } else {
            topfr.fr_width = width;
            topfr.fr_height = height;

            if dir != b'h' as c_int {
                let mut n = frame_minheight(topfr, nowin());
                if row + height >= g_st().cmdline_row && opt_vars().p_ls == 0 {
                    extra_sep = STATUS_HEIGHT;
                } else if global_stl_height() > 0 {
                    extra_sep = 1;
                } else {
                    extra_sep = 0;
                }
                totwincount = get_maximum_wincount(topfr, n + extra_sep);
                has_next_curwin = frame_has_win(topfr, next_curwin);

                let m = frame_minheight(topfr, next_curwin);
                room = height - m;
                if room < 0 {
                    next_curwin_size = opt_vars().p_wh as c_int + room;
                    room = 0;
                } else {
                    next_curwin_size = -1;
                    for fr in frame_iter_ptr(topfr.fr_child) {
                        if frame_fixed_height(fr) {
                            n = frame_minheight(&mut *fr, nowin());
                            let mut new_size = (*fr).fr_height;
                            if frame_has_win(&*fr, next_curwin) {
                                room += opt_vars().p_wh as c_int - opt_vars().p_wmh as c_int;
                                next_curwin_size = 0;
                                new_size = new_size.max(opt_vars().p_wh as c_int);
                            } else {
                                totwincount -= get_maximum_wincount(
                                    &mut *fr,
                                    n + if (*fr).fr_next.is_null() {
                                        extra_sep
                                    } else {
                                        0
                                    },
                                );
                            }
                            room -= new_size - n;
                            if room < 0 {
                                new_size += room;
                                room = 0;
                            }
                            (*fr).fr_newheight = new_size;
                        }
                    }
                    if next_curwin_size == -1 {
                        if !has_next_curwin {
                            next_curwin_size = 0;
                        } else if totwincount > 1
                            && OptInt::from((room + (totwincount - 2)) / (totwincount - 1))
                                > opt_vars().p_wh
                        {
                            next_curwin_size = ((OptInt::from(room)
                                + opt_vars().p_wh
                                + OptInt::from(totwincount - 1) * opt_vars().p_wmh
                                + OptInt::from(totwincount - 1))
                                as c_int)
                                / totwincount;
                            room -= next_curwin_size - opt_vars().p_wh as c_int;
                        } else {
                            next_curwin_size = opt_vars().p_wh as c_int;
                        }
                    }
                }

                if has_next_curwin {
                    totwincount -= 1;
                }
            }

            for fr in frame_iter_ptr(topfr.fr_child) {
                let mut wincount: c_int = 1;
                let new_size: c_int;
                if (*fr).fr_next.is_null() {
                    new_size = height;
                } else if dir == b'h' as c_int {
                    new_size = (*fr).fr_height;
                } else if frame_fixed_height(fr) {
                    new_size = (*fr).fr_newheight;
                    wincount = 0;
                } else {
                    let n = frame_minheight(&mut *fr, nowin());
                    wincount = get_maximum_wincount(
                        &mut *fr,
                        n + if (*fr).fr_next.is_null() {
                            extra_sep
                        } else {
                            0
                        },
                    );
                    let m = frame_minheight(&mut *fr, next_curwin);
                    let hnc = has_next_curwin && frame_has_win(&*fr, next_curwin);
                    if hnc {
                        wincount -= 1;
                    }
                    let mut ns = if totwincount == 0 {
                        room
                    } else {
                        (wincount * room + (totwincount / 2)) / totwincount
                    };
                    if hnc {
                        next_curwin_size -= opt_vars().p_wh as c_int - (m - n);
                        ns += next_curwin_size;
                        room -= ns - next_curwin_size;
                    } else {
                        room -= ns;
                    }
                    new_size = ns + n;
                }
                if !current
                    || dir != b'h' as c_int
                    || !topfr.fr_parent.is_null()
                    || new_size != (*fr).fr_height
                    || frame_has_win(&*fr, next_curwin)
                {
                    win_equal_rec(
                        next_curwin,
                        current,
                        &mut *fr,
                        dir,
                        col,
                        row,
                        width,
                        new_size,
                    );
                }
                row += new_size;
                height -= new_size;
                totwincount -= wincount;
            }
        }
    }
}

pub fn leaving_window(win: &mut WinInfoInfo) {
    {
        if !bt_prompt(unsafe { win.w_buffer.as_ref() }) || is_ctx_win(&raw mut *win) {
            return;
        }

        win.buf_mut().b_prompt_insert = g_st().restart_edit;
        if g_st().restart_edit != NUL as c_int && g_st().mode_displayed {
            g_st().clear_cmdline = true;
        }
        g_st().restart_edit = NUL as c_int;

        if (g_st().state_global & MODE_INSERT) != 0 && !g_st().ins_global.stop_insert_mode {
            g_st().ins_global.stop_insert_mode = true;
            if win.buf_mut().b_prompt_insert == NUL as c_int {
                win.buf_mut().b_prompt_insert = b'A' as c_int;
            }
        }
    }
}

pub fn entering_window(win: &mut WinInfoInfo) {
    {
        if !bt_prompt(unsafe { win.w_buffer.as_ref() }) || is_ctx_win(&raw mut *win) {
            return;
        }

        if win.buf_mut().b_prompt_insert != NUL as c_int {
            g_st().ins_global.stop_insert_mode = false;
        }

        if (g_st().state_global & MODE_INSERT) == 0 {
            g_st().restart_edit = win.buf_mut().b_prompt_insert;
        }
    }
}

pub fn win_init_empty(wp: &mut WinInfoInfo) {
    redraw_later(Some(&mut *wp), UPD_NOT_VALID);
    wp.w_lines_valid = 0;
    wp.w_cursor.lnum = 1;
    wp.w_cursor.col = 0;
    wp.w_curswant = 0;
    wp.w_cursor.coladd = 0;
    wp.w_pcmark.lnum = 1;
    wp.w_pcmark.col = 0;
    wp.w_prev_pcmark.lnum = 0;
    wp.w_prev_pcmark.col = 0;
    wp.w_topline = 1;
    wp.w_topfill = 0;
    wp.w_botline = 2;
    wp.w_valid = 0;
    wp.w_s_owned = None;
}

pub fn curwin_init() {
    {
        win_init_empty(curwin_mut());
    }
}

pub fn close_windows(buf: *mut FileBuf, keep_curwin: bool) {
    unsafe {
        g_st().redrawing_disabled += 1;

        let mut wp = g_st().lastwin;
        while !wp.is_null()
            && (is_ctx_win(g_st().lastwin) || !one_window(&mut *wp, ptr::null_mut()))
        {
            if ptr::eq((*wp).w_buffer, buf)
                && (!keep_curwin || !ptr::eq(wp, curwin_ptr()))
                && !(win_locked(&mut *wp) != 0 || (*wp).buf_mut().b_locked > 0)
            {
                if window_layout_locked(Cmdidx::CmdSize) {
                    g_st().redrawing_disabled -= 1;
                    return;
                }
                if !win_close(wp, false, false) {
                    break;
                }
                wp = g_st().lastwin;
            } else {
                wp = (*wp).w_prev;
            }
        }

        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut nexttp = (*tp).tp_next;
            if !ptr::eq(tp, g_st().curtab) {
                let mut wp2 = (*tp).tp_lastwin;
                while !wp2.is_null() {
                    if ptr::eq((*wp2).w_buffer, buf)
                        && !(win_locked(&mut *wp2) != 0 || (*wp2).buf_mut().b_locked > 0)
                    {
                        if window_layout_locked(Cmdidx::CmdSize) {
                            g_st().redrawing_disabled -= 1;
                            return;
                        }
                        if !win_close_othertab(wp2, 0, tp, false) {
                            g_st().redrawing_disabled -= 1;
                            return;
                        }
                        nexttp = g_st().first_tabpage;
                        break;
                    }
                    wp2 = (*wp2).w_prev;
                }
            }
            tp = nexttp;
        }

        g_st().redrawing_disabled -= 1;
    }
}

pub fn last_window(win: &mut WinInfoInfo) -> bool {
    unsafe { one_window(win, ptr::null_mut()) && (*g_st().first_tabpage).tp_next.is_null() }
}

pub fn one_window(win: &mut WinInfoInfo, tp: *mut TabPageInfo) -> bool {
    unsafe {
        let first = if !tp.is_null() {
            (*tp).tp_firstwin
        } else {
            g_st().firstwin
        };
        debug_assert!((tp.is_null() || !ptr::eq(tp, g_st().curtab)) && !(*first).w_floating);
        ptr::eq(first, (&raw mut *win)) && (win.w_next.is_null() || (*win.w_next).w_floating)
    }
}

fn can_close_floating_windows(tp: *mut TabPageInfo) -> bool {
    unsafe {
        debug_assert!(
            !ptr::eq(tp, g_st().curtab) && (!tp.is_null() || !is_ctx_win(g_st().lastwin))
        );
        let mut wp = if !tp.is_null() {
            (*tp).tp_lastwin
        } else {
            g_st().lastwin
        };
        while (*wp).w_floating {
            let buf = (*wp).w_buffer;
            let need_hide = buf_is_changed_fn(&mut *buf) && (*buf).b_nwindows <= 1;
            if need_hide && !buf_hide(&*buf) {
                return false;
            }
            wp = (*wp).w_prev;
        }
        true
    }
}

fn close_last_window_tabpage(
    win: &mut WinInfoInfo,
    mut free_buf: bool,
    prev_curtab: &mut TabPageInfo,
) -> bool {
    {
        if !one_window_macro() {
            return false;
        }

        let old_curbuf = g_st().curbuf;
        let save_lastused = g_st().lastused_tabpage;

        let term = if !win.w_buffer.is_null() {
            win.buf_mut().terminal
        } else {
            ptr::null_mut()
        };
        if !term.is_null() {
            free_buf = false;
        }

        goto_tabpage_tp(alt_tabpage(), false, !win.w_buffer.is_null());

        if !ptr::eq(g_st().curtab, prev_curtab)
            && valid_tabpage(&raw mut *prev_curtab)
            && ptr::eq(prev_curtab.tp_firstwin, (&raw mut *win))
        {
            win_close_othertab(
                (&raw mut *win),
                c_int::from(free_buf),
                &raw mut *prev_curtab,
                false,
            );
        }

        if valid_tabpage(save_lastused) && !ptr::eq(save_lastused, g_st().curtab) {
            g_st().lastused_tabpage = save_lastused;
        }
        entering_window(curwin_mut());

        apply_autocmds(
            EVENT_WINENTER,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        apply_autocmds(
            EVENT_TABENTER,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        if !ptr::eq(old_curbuf, g_st().curbuf) {
            apply_autocmds(
                EVENT_BUFENTER,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }
        true
    }
}

fn win_close_buffer(win: &mut WinInfoInfo, action: c_int, abort_if_last: bool) -> bool {
    {
        if !win.w_buffer.is_null() {
            reset_synblock(win);
        }

        if !win.w_buffer.is_null()
            && bt_quickfix(unsafe { win.w_buffer.as_ref() })
            && win.buf_mut().b_nwindows == 1
        {
            win.buf_mut().b_p_bl = 0;
        }

        let mut retval = false;
        if !win.w_buffer.is_null() {
            let mut bufref: Bufref = Bufref::new();
            set_bufref(&mut bufref, g_st().curbuf);
            win.w_locked += 1;
            retval = close_buffer(
                (&raw mut *win),
                win.w_buffer,
                action,
                abort_if_last,
                true,
                true,
            );
            if win_valid_any_tab(&raw mut *win) {
                win.w_locked -= 1;
            }

            if !bufref_valid(Some(&bufref)) {
                g_st().curbuf = g_st().firstbuf;
            }
        }

        retval
    }
}

fn win_unclose_buffer(win: &mut WinInfoInfo, _bufref: &mut Bufref) {
    unsafe {
        if win.w_buffer.is_null() {
            win.w_buffer = g_st().firstbuf;
            (*g_st().firstbuf).b_nwindows += 1;
            if ptr::eq((&raw mut *win), curwin_ptr()) {
                g_st().curbuf = curwin_mut().w_buffer;
            }
            win_init_empty(win);
        }
    }
}

pub fn win_close(win: *mut WinInfoInfo, free_buf: bool, force: bool) -> bool {
    unsafe {
        let prev_curtab = g_st().curtab;
        let win_frame = if (*win).w_floating {
            ptr::null_mut()
        } else {
            (*(*win).w_frame).fr_parent
        };
        let had_diffmode = (*win).w_onebuf_opt.wo_diff != 0;

        if last_window(&mut *win) {
            emsg(gettext(E_CANNOT_CLOSE_LAST_WINDOW));
            return false;
        }
        if !(*win).w_floating && window_layout_locked(Cmdidx::CmdClose) {
            return false;
        }

        if win_locked(&mut *win) != 0
            || (!(*win).w_buffer.is_null() && (*win).buf_mut().b_locked > 0)
        {
            return false;
        }
        if is_ctx_win(win) {
            emsg(gettext(E_AUTOCMD_CLOSE));
            return false;
        }
        if lastwin_mut().w_floating && one_window(&mut *win, ptr::null_mut()) {
            if is_ctx_win(g_st().lastwin) {
                emsg(gettext(
                    c"E814: Cannot close window, only autocmd window would remain",
                ));
                return false;
            }
            if force || can_close_floating_windows(ptr::null_mut()) {
                while lastwin_mut().w_floating {
                    if !win_close(g_st().lastwin, !buf_hide(lastwin_mut().buf_mut()), true) {
                        return false;
                    }
                }
                if !win_valid_any_tab(win) {
                    return false;
                }
                if last_window(&mut *win) {
                    emsg(gettext(E_CANNOT_CLOSE_LAST_WINDOW));
                    return false;
                }
            } else {
                emsg(E_FLOATONLY);
                return false;
            }
        }

        if close_last_window_tabpage(&mut *win, free_buf, &mut *prev_curtab) {
            return false;
        }

        let mut help_window = false;
        let mut quickfix_window = false;

        if bt_help(((*win).w_buffer).as_ref()) {
            help_window = true;
        } else {
            clear_snapshot(curtab_mut(), SNAP_HELP_IDX as c_int);
        }

        if bt_quickfix(((*win).w_buffer).as_ref()) {
            quickfix_window = true;
        } else {
            clear_snapshot(curtab_mut(), SNAP_QUICKFIX_IDX as c_int);
        }

        let mut other_buffer = false;

        if ptr::eq(win, curwin_ptr()) {
            leaving_window(curwin_mut());

            let wp = if (*win).w_floating {
                win_float_find_altwin(&*win, None)
            } else {
                frame2win(win_altframe(&mut *win, ptr::null_mut()))
            };

            if !ptr::eq((*wp).w_buffer, g_st().curbuf) {
                reset_visual_and_resel();

                other_buffer = true;
                if !win_valid(win) {
                    return false;
                }
                (*win).w_locked += 1;
                apply_autocmds(
                    EVENT_BUFLEAVE,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                );
                if !win_valid(win) {
                    return false;
                }
                (*win).w_locked -= 1;
                if last_window(&mut *win) {
                    return false;
                }
            }
            (*win).w_locked += 1;
            apply_autocmds(
                EVENT_WINLEAVE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            if !win_valid(win) {
                return false;
            }
            (*win).w_locked -= 1;
            if last_window(&mut *win) {
                return false;
            }
            if aborting() {
                return false;
            }
        }

        do_autocmd_winclosed(&mut *win);
        if !win_valid_any_tab(win) {
            return true;
        }

        let mut bufref: Bufref = Bufref::new();
        set_bufref(&mut bufref, (*win).w_buffer);

        win_close_buffer(&mut *win, if free_buf { DOBUF_UNLOAD } else { 0 }, true);

        if win_valid(win)
            && (*win).w_buffer.is_null()
            && !(*win).w_floating
            && last_window(&mut *win)
        {
            if curwin_mut().w_buffer.is_null() {
                curwin_mut().w_buffer = g_st().curbuf;
            }
            getout(0);
        }
        if !ptr::eq(g_st().curtab, prev_curtab)
            && win_valid_any_tab(win)
            && (*win).w_buffer.is_null()
        {
            win_close_othertab(win, 0, prev_curtab, force);
            return false;
        }

        if !win_valid(win) {
            return false;
        }
        if one_window(&mut *win, ptr::null_mut())
            && ((*g_st().first_tabpage).tp_next.is_null() || lastwin_mut().w_floating)
        {
            if !(*g_st().first_tabpage).tp_next.is_null() {
                emsg(E_FLOATONLY);
            }
            win_unclose_buffer(&mut *win, &mut bufref);
            return false;
        }
        if close_last_window_tabpage(&mut *win, free_buf, &mut *prev_curtab) {
            return false;
        }

        win_st().split_disallowed += 1;

        let was_floating = (*win).w_floating;
        if ui_has(K_UIMULTIGRID) {
            ui_call_win_close(Integer::from((*win).w_grid_alloc.handle));
        }

        if (*win).w_floating {
            ui_comp_remove_grid(&mut (*win).w_grid_alloc);
            debug_assert!(!g_st().first_tabpage.is_null());
            if (*win).w_config.external {
                for tp in for_all_tabpages_ptr() {
                    if !ptr::eq(tp, g_st().curtab) && ptr::eq((*tp).tp_curwin, win) {
                        (*tp).tp_curwin = (*tp).tp_firstwin;
                    }
                }
            }
        }

        set_bufref(&mut bufref, (*win).w_buffer);

        if !(*win).w_buffer.is_null() {
            (*win).buf_mut().b_nwindows -= 1;
        }

        let had_cmdline_ruler =
            opt_vars().p_ru != 0 && ptr::eq(win, curwin_ptr()) && (*win).w_status_height == 0;
        let mut dir: c_int = 0;
        let mut wp = win_free_mem(win, &mut dir, ptr::null_mut());

        if help_window || quickfix_window {
            let prev_win = get_snapshot_curwin(if help_window {
                SNAP_HELP_IDX as c_int
            } else {
                SNAP_QUICKFIX_IDX as c_int
            });
            if win_valid(prev_win) {
                wp = prev_win;
            }
        }

        let mut close_curwin = false;

        if ptr::eq(win, curwin_ptr()) {
            set_curwin(wp);
            if (*wp).w_onebuf_opt.wo_pvw != 0 || bt_quickfix(((*wp).w_buffer).as_ref()) {
                loop {
                    if (*wp).w_next.is_null() {
                        wp = g_st().firstwin;
                    } else {
                        wp = (*wp).w_next;
                    }
                    if ptr::eq(wp, curwin_ptr()) {
                        break;
                    }
                    if (*wp).w_onebuf_opt.wo_pvw == 0
                        && !bt_quickfix(((*wp).w_buffer).as_ref())
                        && !((*wp).w_floating && ((*wp).w_config.hide || !(*wp).w_config.focusable))
                    {
                        set_curwin(wp);
                        break;
                    }
                }
            }
            g_st().curbuf = curwin_mut().w_buffer;
            close_curwin = true;

            check_cursor(curwin_mut());
        }

        if !was_floating {
            last_status(false);

            if !curwin_mut().w_floating
                && opt_vars().p_ea != 0
                && (crate::memory::cstr_first_byte(opt_vars().p_ead) == b'b'
                    || c_int::from(crate::memory::cstr_first_byte(opt_vars().p_ead)) == dir)
            {
                win_equal(
                    curwin_ptr(),
                    ptr::eq((*curwin_mut().w_frame).fr_parent, win_frame),
                    dir,
                );
            } else {
                win_comp_pos();
                win_fix_scroll(false);
            }
        } else if had_cmdline_ruler && (*wp).w_status_height > 0 {
            g_st().redraw_cmdline = true;
        }
        if !bufref.br_buf.is_null()
            && bufref_valid(Some(&bufref))
            && !(*bufref.br_buf).terminal.is_null()
        {
            terminal_check_size(&mut *(*bufref.br_buf).terminal);
        }

        if close_curwin {
            win_enter_ext(
                wp,
                WEE_CURWIN_INVALID | WEE_TRIGGER_ENTER_AUTOCMDS | WEE_TRIGGER_LEAVE_AUTOCMDS,
            );
            if other_buffer {
                apply_autocmds(
                    EVENT_BUFENTER,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                );
            }
        }

        if one_window_macro()
            && curwin_mut().w_locked != 0
            && curbuf_mut().b_locked_split != 0
            && !(*g_st().first_tabpage).tp_next.is_null()
        {
            apply_autocmds(
                EVENT_TABLEAVE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }

        win_st().split_disallowed -= 1;

        if help_window || quickfix_window {
            restore_snapshot(
                if help_window {
                    SNAP_HELP_IDX as c_int
                } else {
                    SNAP_QUICKFIX_IDX as c_int
                },
                c_int::from(close_curwin),
            );
        }

        if diffopt_closeoff() && had_diffmode && ptr::eq(g_st().curtab, prev_curtab) {
            let mut diffcount: c_int = 0;
            for dwin in for_all_windows_mut() {
                if dwin.w_onebuf_opt.wo_diff != 0 {
                    diffcount += 1;
                }
            }
            if diffcount == 1 {
                do_cmdline_cmd(c"diffoff!");
            }
        }

        curwin_mut().w_pos_changed = true;
        if !was_floating {
            redraw_all_later(UPD_NOT_VALID);
        }
        true
    }
}

fn trigger_winnewpre() {
    window_layout_lock();
    apply_autocmds(
        EVENT_WINNEWPRE,
        ptr::null_mut(),
        ptr::null_mut(),
        false,
        ptr::null_mut(),
    );
    window_layout_unlock();
}

fn do_autocmd_winclosed(win: &mut WinInfoInfo) {
    {
        if win_st().winclosed_recursive || !has_event(EVENT_WINCLOSED) {
            return;
        }
        win_st().winclosed_recursive = true;
        let mut winid = [0 as c_char; NUMBUFLEN];
        vim_snprintf!(winid.as_mut_ptr(), winid.len(), c"%d", win.handle,);
        apply_autocmds_win(
            EVENT_WINCLOSED,
            winid.as_mut_ptr(),
            winid.as_mut_ptr(),
            false,
            win.w_buffer,
            Some(win),
        );
        win_st().winclosed_recursive = false;
    }
}

pub fn trigger_tabclosedpre(tp: *mut TabPageInfo) {
    let ptp = g_st().curtab;

    if !has_event(EVENT_TABCLOSEDPRE) || win_st().tabclosedpre_recursive {
        return;
    }

    if valid_tabpage(tp) {
        goto_tabpage_tp(tp, false, false);
    }
    win_st().tabclosedpre_recursive = true;
    window_layout_lock();
    apply_autocmds(
        EVENT_TABCLOSEDPRE,
        ptr::null_mut(),
        ptr::null_mut(),
        false,
        ptr::null_mut(),
    );
    window_layout_unlock();
    win_st().tabclosedpre_recursive = false;
    if valid_tabpage(ptp) {
        goto_tabpage_tp(ptp, false, false);
    } else {
        goto_tabpage_tp(g_st().first_tabpage, false, false);
    }
}

pub fn win_close_othertab(
    win: *mut WinInfoInfo,
    free_buf: c_int,
    tp: *mut TabPageInfo,
    force: bool,
) -> bool {
    unsafe {
        debug_assert!(!ptr::eq(tp, g_st().curtab));

        if window_layout_locked(Cmdidx::CmdSize) {
            return false;
        }
        if win_locked(&mut *win) != 0
            || (!(*win).w_buffer.is_null() && (*win).buf_mut().b_locked > 0)
        {
            return false;
        }
        if is_ctx_win(win) {
            emsg(gettext(E_AUTOCMD_CLOSE));
            return false;
        }

        if (*(*tp).tp_lastwin).w_floating && one_window(&mut *win, tp) {
            if force || can_close_floating_windows(tp) {
                loop {
                    if !(*(*tp).tp_lastwin).w_floating {
                        break;
                    }
                    if !win_close_othertab(
                        (*tp).tp_lastwin,
                        c_int::from(!buf_hide(&*(*(*tp).tp_lastwin).w_buffer)),
                        tp,
                        true,
                    ) {
                        return false;
                    }
                }
                if !win_valid_any_tab(win) {
                    return false;
                }
            } else {
                emsg(E_FLOATONLY);
                return false;
            }
        }

        if !(*win).w_buffer.is_null() {
            do_autocmd_winclosed(&mut *win);
            if !win_valid_any_tab(win) {
                return false;
            }
        }

        if ptr::eq((*tp).tp_firstwin, (*tp).tp_lastwin) && !(*tp).tp_did_tabclosedpre {
            trigger_tabclosedpre(tp);
            if !win_valid_any_tab(win) {
                return false;
            }
        }

        let mut bufref: Bufref = Bufref::new();
        set_bufref(&mut bufref, (*win).w_buffer);

        if !(*win).w_buffer.is_null() {
            close_buffer(
                win,
                (*win).w_buffer,
                if free_buf != 0 { DOBUF_UNLOAD } else { 0 },
                false,
                true,
                true,
            );
        }

        if !valid_tabpage(tp) || ptr::eq(tp, g_st().curtab) || !tabpage_win_valid(tp, win) {
            if win_valid_any_tab(win) {
                win_unclose_buffer(&mut *win, &mut bufref);
            }
            return false;
        }
        if (*(*tp).tp_lastwin).w_floating && one_window(&mut *win, tp) {
            emsg(E_FLOATONLY);
            if win_valid_any_tab(win) {
                win_unclose_buffer(&mut *win, &mut bufref);
            }
            return false;
        }

        let mut free_tp_idx: c_int = 0;

        if ptr::eq((*tp).tp_firstwin, (*tp).tp_lastwin) {
            free_tp_idx = tabpage_index(tp);
            let h = tabline_height();

            if ptr::eq(tp, g_st().first_tabpage) {
                g_st().first_tabpage = (*tp).tp_next;
            } else {
                let mut ptp = g_st().first_tabpage;
                while !ptp.is_null() && !ptr::eq((*ptp).tp_next, tp) {
                    ptp = (*ptp).tp_next;
                }
                if ptp.is_null() {
                    internal_error(c"win_close_othertab()");
                    return false;
                }
                (*ptp).tp_next = (*tp).tp_next;
            }
            g_st().redraw_tabline = true;
            if h != tabline_height() {
                win_new_screen_rows();
            }
        }

        if ui_has(K_UIMULTIGRID) {
            ui_call_win_close(Integer::from((*win).w_grid_alloc.handle));
        }
        if (*win).w_floating {
            ui_comp_remove_grid(&mut (*win).w_grid_alloc);
        }

        set_bufref(&mut bufref, (*win).w_buffer);

        if !(*win).w_buffer.is_null() {
            (*win).buf_mut().b_nwindows -= 1;
        }

        let mut dir: c_int = 0;
        win_free_mem(win, &mut dir, tp);

        if !bufref.br_buf.is_null()
            && bufref_valid(Some(&bufref))
            && !(*bufref.br_buf).terminal.is_null()
        {
            terminal_check_size(&mut *(*bufref.br_buf).terminal);
        }
        if free_tp_idx > 0 {
            free_tabpage(tp);

            if has_event(EVENT_TABCLOSED) {
                let mut prev_idx = [0 as c_char; NUMBUFLEN];
                vim_snprintf!(prev_idx.as_mut_ptr(), NUMBUFLEN, c"%i", free_tp_idx,);
                apply_autocmds(
                    EVENT_TABCLOSED,
                    prev_idx.as_mut_ptr(),
                    prev_idx.as_mut_ptr(),
                    false,
                    if !bufref.br_buf.is_null() && bufref_valid(Some(&bufref)) {
                        bufref.br_buf
                    } else {
                        g_st().curbuf
                    },
                );
            }
        }
        true
    }
}

fn win_free_mem(win: *mut WinInfoInfo, dirp: &mut c_int, tp: *mut TabPageInfo) -> *mut WinInfoInfo {
    unsafe {
        let wp: *mut WinInfoInfo;
        let win_tp = if tp.is_null() { g_st().curtab } else { tp };

        if !(*win).w_floating {
            let frp = (*win).w_frame;
            wp = winframe_remove(&mut *win, &mut *dirp, tp, ptr::null_mut());
            drop(Box::from_raw(frp));
        } else {
            *dirp = b'h' as c_int;
            wp = win_float_find_altwin(&*win, tp.as_ref());
        }
        win_free(win, tp);

        if ptr::eq(win, (*win_tp).tp_curwin) {
            (*win_tp).tp_curwin = wp;
        }
        if ptr::eq(win, g_st().cmdline_win) {
            g_st().cmdline_win = ptr::null_mut();
        }

        wp
    }
}

pub fn win_free_all() {
    unsafe {
        g_st().cmdwin_type = 0;
        g_st().cmdwin_buf = ptr::null_mut();

        while !(*g_st().first_tabpage).tp_next.is_null() {
            tabpage_close(1);
        }

        while !g_st().lastwin.is_null() && lastwin_mut().w_floating {
            let wp = g_st().lastwin;
            win_remove(lastwin_mut(), ptr::null_mut());
            let mut dummy: c_int = 0;
            win_free_mem(wp, &mut dummy, ptr::null_mut());
            for item in g_st().ctx_win_vec.iter_mut() {
                if ptr::eq(item.cw_win, wp) {
                    item.cw_win = ptr::null_mut();
                }
            }
        }

        for i in 0..g_st().ctx_win_vec.len() {
            let cw = g_st().ctx_win_vec.as_slice()[i].cw_win;
            if !cw.is_null() {
                let mut dummy: c_int = 0;
                win_free_mem(cw, &mut dummy, ptr::null_mut());
                g_st().ctx_win_vec.as_mut_slice()[i].cw_win = ptr::null_mut();
            }
        }

        g_st().ctx_win_vec = Vec::new();

        while !g_st().firstwin.is_null() {
            let mut dummy: c_int = 0;
            win_free_mem(g_st().firstwin, &mut dummy, ptr::null_mut());
        }

        set_curwin(ptr::null_mut());
    }
}

pub fn winframe_remove(
    win: &mut WinInfoInfo,
    dirp: &mut c_int,
    tp: *mut TabPageInfo,
    unflat_altfr: *mut *mut Frame,
) -> *mut WinInfoInfo {
    unsafe {
        let mut altfr: *mut Frame = ptr::null_mut();
        let wp = winframe_find_altwin(win, dirp, tp, &raw mut altfr);
        if wp.is_null() {
            return ptr::null_mut();
        }

        let frp_close = &mut *win.w_frame;

        win_st().frame_locked += 1;

        let topleft = frame2win(frp_close.fr_parent);
        let mut row = topleft.w_winrow;
        let mut col = topleft.w_wincol;

        if win.w_vsep_width == 0
            && (*frp_close.fr_parent).fr_layout == FR_ROW
            && !frp_close.fr_prev.is_null()
        {
            frame_set_vsep(frp_close.fr_prev, false);
        }

        frame_remove(&mut *frp_close);

        if *dirp == b'v' as c_int {
            frame_new_height(
                altfr,
                (*altfr).fr_height + frp_close.fr_height,
                ptr::eq(altfr, frp_close.fr_next),
                !frame_fixed_height(altfr),
                false,
            );
        } else {
            debug_assert!(*dirp == b'h' as c_int);
            frame_new_width(
                &mut *altfr,
                (*altfr).fr_width + frp_close.fr_width,
                ptr::eq(altfr, frp_close.fr_next),
                !frame_fixed_height(altfr),
            );
        }

        if !ptr::eq(altfr, frp_close.fr_prev) {
            frame_comp_pos(&mut *frp_close.fr_parent, &mut row, &mut col);
        }

        if unflat_altfr.is_null() {
            frame_flatten(altfr);
        } else {
            *unflat_altfr = altfr;
        }

        win_st().frame_locked -= 1;

        wp
    }
}

pub fn winframe_find_altwin(
    win: &mut WinInfoInfo,
    dirp: &mut c_int,
    tp: *mut TabPageInfo,
    altfr: *mut *mut Frame,
) -> *mut WinInfoInfo {
    unsafe {
        debug_assert!(tp.is_null() || !ptr::eq(tp, g_st().curtab));

        if one_window(win, tp) {
            return ptr::null_mut();
        }

        let frp_close = win.w_frame;
        let mut frp2: *mut Frame = win_altframe(win, tp);
        let mut wp: *mut WinInfoInfo = frame2win(frp2);

        if (*(*frp_close).fr_parent).fr_layout == FR_COL {
            if !(*frp2).fr_win.is_null() && (*(*frp2).fr_win).w_onebuf_opt.wo_wfh != 0 {
                let mut frp = (*frp_close).fr_prev;
                let mut frp3 = (*frp_close).fr_next;
                while !frp.is_null() || !frp3.is_null() {
                    if !frp.is_null() {
                        if !frame_fixed_height(frp) {
                            frp2 = frp;
                            wp = frame2win(frp2);
                            break;
                        }
                        frp = (*frp).fr_prev;
                    }
                    if !frp3.is_null() {
                        if !(*frp3).fr_win.is_null() && (*(*frp3).fr_win).w_onebuf_opt.wo_wfh == 0 {
                            frp2 = frp3;
                            wp = (*frp3).fr_win;
                            break;
                        }
                        frp3 = (*frp3).fr_next;
                    }
                }
            }
            *dirp = b'v' as c_int;
        } else {
            if !(*frp2).fr_win.is_null() && (*(*frp2).fr_win).w_onebuf_opt.wo_wfw != 0 {
                let mut frp = (*frp_close).fr_prev;
                let mut frp3 = (*frp_close).fr_next;
                while !frp.is_null() || !frp3.is_null() {
                    if !frp.is_null() {
                        if !frame_fixed_width(frp) {
                            frp2 = frp;
                            wp = frame2win(frp2);
                            break;
                        }
                        frp = (*frp).fr_prev;
                    }
                    if !frp3.is_null() {
                        if !(*frp3).fr_win.is_null() && (*(*frp3).fr_win).w_onebuf_opt.wo_wfw == 0 {
                            frp2 = frp3;
                            wp = (*frp3).fr_win;
                            break;
                        }
                        frp3 = (*frp3).fr_next;
                    }
                }
            }
            *dirp = b'h' as c_int;
        }

        debug_assert!(!ptr::eq(wp, (&raw mut *win)) && !ptr::eq(frp2, frp_close));
        if !altfr.is_null() {
            *altfr = frp2;
        }

        wp
    }
}

fn frame_flatten(mut frp: *mut Frame) {
    unsafe {
        if !(*frp).fr_next.is_null() || !(*frp).fr_prev.is_null() {
            return;
        }

        (*(*frp).fr_parent).fr_layout = (*frp).fr_layout;
        (*(*frp).fr_parent).fr_child = (*frp).fr_child;
        let mut frp2 = (*frp).fr_child;
        while !frp2.is_null() {
            (*frp2).fr_parent = (*frp).fr_parent;
            frp2 = (*frp2).fr_next;
        }
        (*(*frp).fr_parent).fr_win = (*frp).fr_win;
        if !(*frp).fr_win.is_null() {
            (*(*frp).fr_win).w_frame = (*frp).fr_parent;
        }
        frp2 = (*frp).fr_parent;
        if ptr::eq((*g_st().topframe).fr_child, frp) {
            (*g_st().topframe).fr_child = frp2;
        }
        drop(Box::from_raw(frp));

        frp = (*frp2).fr_parent;
        if !frp.is_null() && (*frp).fr_layout == (*frp2).fr_layout {
            if ptr::eq((*frp).fr_child, frp2) {
                (*frp).fr_child = (*frp2).fr_child;
            }
            debug_assert!(!(*frp2).fr_child.is_null());
            (*(*frp2).fr_child).fr_prev = (*frp2).fr_prev;
            if !(*frp2).fr_prev.is_null() {
                (*(*frp2).fr_prev).fr_next = (*frp2).fr_child;
            }
            let mut frp3 = (*frp2).fr_child;
            loop {
                (*frp3).fr_parent = frp;
                if (*frp3).fr_next.is_null() {
                    (*frp3).fr_next = (*frp2).fr_next;
                    if !(*frp2).fr_next.is_null() {
                        (*(*frp2).fr_next).fr_prev = frp3;
                    }
                    break;
                }
                frp3 = (*frp3).fr_next;
            }
            if ptr::eq((*g_st().topframe).fr_child, frp2) {
                (*g_st().topframe).fr_child = frp;
            }
            drop(Box::from_raw(frp2));
        }
    }
}

pub fn winframe_restore(wp: &mut WinInfoInfo, dir: c_int, unflat_altfr: &mut Frame) {
    unsafe {
        let frp = &mut *wp.w_frame;

        if !frp.fr_prev.is_null() {
            frame_append(&mut *frp.fr_prev, &mut *frp);
        } else {
            frame_insert(&mut *frp.fr_next, &mut *frp);
        }

        if wp.w_vsep_width == 0 && (*frp.fr_parent).fr_layout == FR_ROW && !frp.fr_prev.is_null() {
            frame_set_vsep(frp.fr_prev, true);
        }

        if (*frp.fr_parent).fr_layout == FR_COL && !frp.fr_prev.is_null() {
            if global_stl_height() == 0 && wp.w_status_height == 0 {
                frame_add_statusline(frp.fr_prev);
            } else if global_stl_height() > 0 && wp.w_hsep_height == 0 {
                frame_add_hsep(frp.fr_prev);
            }
        }

        if dir == b'v' as c_int {
            frame_new_height(
                &raw mut *unflat_altfr,
                unflat_altfr.fr_height - frp.fr_height,
                ptr::eq(&raw mut *unflat_altfr, frp.fr_next),
                false,
                false,
            );
        } else if dir == b'h' as c_int {
            frame_new_width(
                unflat_altfr,
                unflat_altfr.fr_width - frp.fr_width,
                ptr::eq(unflat_altfr, frp.fr_next),
                false,
            );
        }

        if !ptr::eq(unflat_altfr, frp.fr_prev) {
            let topleft = frame2win(frp.fr_parent);
            let mut row = topleft.w_winrow;
            let mut col = topleft.w_wincol;

            frame_comp_pos(&mut *frp.fr_parent, &mut row, &mut col);
        }
    }
}

fn win_altframe(win: &mut WinInfoInfo, tp: *mut TabPageInfo) -> &'static mut Frame {
    unsafe {
        debug_assert!(tp.is_null() || !ptr::eq(tp, g_st().curtab));

        if one_window(win, tp) {
            return &mut *(*(*alt_tabpage()).tp_curwin).w_frame;
        }

        let frp = &mut *win.w_frame;

        if frp.fr_prev.is_null() {
            return &mut *frp.fr_next;
        }
        if frp.fr_next.is_null() {
            return &mut *frp.fr_prev;
        }

        let mut target_fr = frp.fr_next;
        let mut other_fr = frp.fr_prev;

        if !frp.fr_parent.is_null() && (*frp.fr_parent).fr_layout == FR_COL && opt_vars().p_sb != 0
        {
            target_fr = frp.fr_prev;
            other_fr = frp.fr_next;
        }

        if !frp.fr_parent.is_null() && (*frp.fr_parent).fr_layout == FR_ROW && opt_vars().p_spr != 0
        {
            target_fr = frp.fr_prev;
            other_fr = frp.fr_next;
        }

        if !frp.fr_parent.is_null() && (*frp.fr_parent).fr_layout == FR_ROW {
            if frame_fixed_width(target_fr) && !frame_fixed_width(other_fr) {
                target_fr = other_fr;
            }
        } else if frame_fixed_height(target_fr) && !frame_fixed_height(other_fr) {
            target_fr = other_fr;
        }

        &mut *target_fr
    }
}

fn alt_tabpage() -> *mut TabPageInfo {
    unsafe {
        if (opt_vars().tcl_flags & K_OPT_TCL_FLAG_USELAST) != 0
            && valid_tabpage(g_st().lastused_tabpage)
        {
            return g_st().lastused_tabpage;
        }

        let forward = !curtab_mut().tp_next.is_null()
            && ((opt_vars().tcl_flags & K_OPT_TCL_FLAG_LEFT) == 0
                || ptr::eq(g_st().curtab, g_st().first_tabpage));

        if forward {
            curtab_mut().tp_next
        } else {
            let mut tp = g_st().first_tabpage;
            while !ptr::eq((*tp).tp_next, g_st().curtab) {
                tp = (*tp).tp_next;
            }
            tp
        }
    }
}

pub fn frame2win(mut frp: *mut Frame) -> &'static mut WinInfoInfo {
    unsafe {
        while (*frp).fr_win.is_null() {
            frp = (*frp).fr_child;
        }
        &mut *(*frp).fr_win
    }
}

fn frame_has_win(frp: &Frame, wp: *const WinInfoInfo) -> bool {
    {
        if frp.fr_layout == FR_LEAF {
            return ptr::eq(frp.fr_win, wp);
        }
        for p in frame_iter_mut(frp.fr_child) {
            if frame_has_win(p, wp) {
                return true;
            }
        }
        false
    }
}

fn is_bottom_win(wp: &mut WinInfoInfo) -> bool {
    unsafe {
        let mut frp = wp.w_frame;
        while !(*frp).fr_parent.is_null() {
            if (*(*frp).fr_parent).fr_layout == FR_COL && !(*frp).fr_next.is_null() {
                return false;
            }
            frp = (*frp).fr_parent;
        }
        true
    }
}

pub fn frame_new_height(
    topfrp: *mut Frame,
    mut height: c_int,
    topfirst: bool,
    wfh: bool,
    set_ch: bool,
) {
    unsafe {
        if (*topfrp).fr_parent.is_null() && set_ch {
            let new_ch = win_st()
                .min_set_ch
                .max(opt_vars().p_ch + OptInt::from((*topfrp).fr_height - height));
            if new_ch != opt_vars().p_ch {
                let save_ch = win_st().min_set_ch;
                set_option_value(K_OPT_CMDHEIGHT, integer_obj(new_ch), 0, true, None);
                win_st().min_set_ch = save_ch;
            }
            height = (rows_avail() as c_int).min(height);
        }
        if !(*topfrp).fr_win.is_null() {
            let wp = (*topfrp).fr_win;
            if is_bottom_win(&mut *wp) {
                (*wp).w_hsep_height = 0;
            }
            win_new_height(
                &mut *wp,
                height - (*wp).w_hsep_height - (*wp).w_status_height,
            );
        } else if (*topfrp).fr_layout == FR_ROW {
            let mut frp: *mut Frame;
            loop {
                frp = (*topfrp).fr_child;
                while !frp.is_null() {
                    frame_new_height(frp, height, topfirst, wfh, set_ch);
                    if (*frp).fr_height > height {
                        height = (*frp).fr_height;
                        break;
                    }
                    frp = (*frp).fr_next;
                }
                if frp.is_null() {
                    break;
                }
            }
        } else {
            let mut frp = (*topfrp).fr_child;
            if wfh {
                while frame_fixed_height(frp) {
                    frp = (*frp).fr_next;
                    if frp.is_null() {
                        return;
                    }
                }
            }
            if !topfirst {
                while !(*frp).fr_next.is_null() {
                    frp = (*frp).fr_next;
                }
                if wfh {
                    while frame_fixed_height(frp) {
                        frp = (*frp).fr_prev;
                    }
                }
            }

            let mut extra_lines = height - (*topfrp).fr_height;
            if extra_lines < 0 {
                while !frp.is_null() {
                    let h = frame_minheight(&mut *frp, ptr::null_mut());
                    if (*frp).fr_height + extra_lines < h {
                        extra_lines += (*frp).fr_height - h;
                        frame_new_height(frp, h, topfirst, wfh, set_ch);
                    } else {
                        frame_new_height(
                            frp,
                            (*frp).fr_height + extra_lines,
                            topfirst,
                            wfh,
                            set_ch,
                        );
                        break;
                    }
                    if topfirst {
                        loop {
                            frp = (*frp).fr_next;
                            if !(wfh && !frp.is_null() && frame_fixed_height(frp)) {
                                break;
                            }
                        }
                    } else {
                        loop {
                            frp = (*frp).fr_prev;
                            if !(wfh && !frp.is_null() && frame_fixed_height(frp)) {
                                break;
                            }
                        }
                    }
                    if frp.is_null() {
                        height -= extra_lines;
                    }
                }
            } else if extra_lines > 0 {
                frame_new_height(frp, (*frp).fr_height + extra_lines, topfirst, wfh, set_ch);
            }
        }
        (*topfrp).fr_height = height;
    }
}

fn frame_fixed_height(mut frp: *mut Frame) -> bool {
    unsafe {
        if !(*frp).fr_win.is_null() {
            return (*(*frp).fr_win).w_onebuf_opt.wo_wfh != 0;
        }
        if (*frp).fr_layout == FR_ROW {
            frp = (*frp).fr_child;
            while !frp.is_null() {
                if frame_fixed_height(frp) {
                    return true;
                }
                frp = (*frp).fr_next;
            }
            return false;
        }

        frp = (*frp).fr_child;
        while !frp.is_null() {
            if !frame_fixed_height(frp) {
                return false;
            }
            frp = (*frp).fr_next;
        }
        true
    }
}

fn frame_fixed_width(mut frp: *mut Frame) -> bool {
    unsafe {
        if !(*frp).fr_win.is_null() {
            return (*(*frp).fr_win).w_onebuf_opt.wo_wfw != 0;
        }
        if (*frp).fr_layout == FR_COL {
            frp = (*frp).fr_child;
            while !frp.is_null() {
                if frame_fixed_width(frp) {
                    return true;
                }
                frp = (*frp).fr_next;
            }
            return false;
        }

        frp = (*frp).fr_child;
        while !frp.is_null() {
            if !frame_fixed_width(frp) {
                return false;
            }
            frp = (*frp).fr_next;
        }
        true
    }
}

fn frame_add_statusline(mut frp: *mut Frame) {
    unsafe {
        if (*frp).fr_layout == FR_LEAF {
            let wp = (*frp).fr_win;
            (*wp).w_status_height = STATUS_HEIGHT;
        } else if (*frp).fr_layout == FR_ROW {
            frp = (*frp).fr_child;
            while !frp.is_null() {
                frame_add_statusline(frp);
                frp = (*frp).fr_next;
            }
        } else {
            debug_assert!((*frp).fr_layout == FR_COL);
            frp = (*frp).fr_child;
            while !(*frp).fr_next.is_null() {
                frp = (*frp).fr_next;
            }
            frame_add_statusline(frp);
        }
    }
}

fn frame_new_width(topfrp: &mut Frame, mut width: c_int, leftfirst: bool, wfw: bool) {
    unsafe {
        if topfrp.fr_layout == FR_LEAF {
            let wp = topfrp.fr_win;
            let mut frp = &raw mut *topfrp;
            while !(*frp).fr_parent.is_null() {
                if (*(*frp).fr_parent).fr_layout == FR_ROW && !(*frp).fr_next.is_null() {
                    break;
                }
                frp = (*frp).fr_parent;
            }
            if (*frp).fr_parent.is_null() {
                (*wp).w_vsep_width = 0;
            }
            win_new_width(&mut *wp, width - (*wp).w_vsep_width);
        } else if topfrp.fr_layout == FR_COL {
            let mut frp: *mut Frame;
            loop {
                frp = topfrp.fr_child;
                while !frp.is_null() {
                    frame_new_width(&mut *frp, width, leftfirst, wfw);
                    if (*frp).fr_width > width {
                        width = (*frp).fr_width;
                        break;
                    }
                    frp = (*frp).fr_next;
                }
                if frp.is_null() {
                    break;
                }
            }
        } else {
            let mut frp = topfrp.fr_child;
            if wfw {
                while frame_fixed_width(frp) {
                    frp = (*frp).fr_next;
                    if frp.is_null() {
                        return;
                    }
                }
            }
            if !leftfirst {
                while !(*frp).fr_next.is_null() {
                    frp = (*frp).fr_next;
                }
                if wfw {
                    while frame_fixed_width(frp) {
                        frp = (*frp).fr_prev;
                    }
                }
            }

            let mut extra_cols = width - topfrp.fr_width;
            if extra_cols < 0 {
                while !frp.is_null() {
                    let w = frame_minwidth(&mut *frp, ptr::null_mut());
                    if (*frp).fr_width + extra_cols < w {
                        extra_cols += (*frp).fr_width - w;
                        frame_new_width(&mut *frp, w, leftfirst, wfw);
                    } else {
                        frame_new_width(&mut *frp, (*frp).fr_width + extra_cols, leftfirst, wfw);
                        break;
                    }
                    if leftfirst {
                        loop {
                            frp = (*frp).fr_next;
                            if !(wfw && !frp.is_null() && frame_fixed_width(frp)) {
                                break;
                            }
                        }
                    } else {
                        loop {
                            frp = (*frp).fr_prev;
                            if !(wfw && !frp.is_null() && frame_fixed_width(frp)) {
                                break;
                            }
                        }
                    }
                    if frp.is_null() {
                        width -= extra_cols;
                    }
                }
            } else if extra_cols > 0 {
                frame_new_width(&mut *frp, (*frp).fr_width + extra_cols, leftfirst, wfw);
            }
        }
        topfrp.fr_width = width;
    }
}

fn frame_set_vsep(mut frp: *const Frame, add: bool) {
    unsafe {
        if (*frp).fr_layout == FR_LEAF {
            let wp = (*frp).fr_win;
            if add && (*wp).w_vsep_width == 0 {
                if (*wp).w_width > 0 {
                    win_new_width(&mut *wp, (*wp).w_width - 1);
                }
                (*wp).w_vsep_width = 1;
            } else if !add && (*wp).w_vsep_width == 1 {
                win_new_width(&mut *wp, (*wp).w_width + 1);
                (*wp).w_vsep_width = 0;
            }
        } else if (*frp).fr_layout == FR_COL {
            frp = (*frp).fr_child;
            while !frp.is_null() {
                frame_set_vsep(frp, add);
                frp = (*frp).fr_next;
            }
        } else {
            debug_assert!((*frp).fr_layout == FR_ROW);
            frp = (*frp).fr_child;
            while !(*frp).fr_next.is_null() {
                frp = (*frp).fr_next;
            }
            frame_set_vsep(frp, add);
        }
    }
}

fn frame_add_hsep(mut frp: *const Frame) {
    unsafe {
        if (*frp).fr_layout == FR_LEAF {
            let wp = (*frp).fr_win;
            (*wp).w_hsep_height = 1;
        } else if (*frp).fr_layout == FR_ROW {
            frp = (*frp).fr_child;
            while !frp.is_null() {
                frame_add_hsep(frp);
                frp = (*frp).fr_next;
            }
        } else {
            debug_assert!((*frp).fr_layout == FR_COL);
            frp = (*frp).fr_child;
            while !(*frp).fr_next.is_null() {
                frp = (*frp).fr_next;
            }
            frame_add_hsep(frp);
        }
    }
}

#[inline]
fn frame_fix_width(wp: &mut WinInfoInfo) {
    unsafe {
        (*wp.w_frame).fr_width = wp.w_width + wp.w_vsep_width;
    }
}

#[inline]
fn frame_fix_height(wp: &mut WinInfoInfo) {
    unsafe {
        (*wp.w_frame).fr_height = wp.w_height + wp.w_hsep_height + wp.w_status_height;
    }
}

fn frame_minheight(topfrp: &mut Frame, next_curwin: *mut WinInfoInfo) -> c_int {
    unsafe {
        let mut m: c_int;
        if !topfrp.fr_win.is_null() {
            let extra_height = (*topfrp.fr_win).w_winbar_height
                + (*topfrp.fr_win).w_hsep_height
                + (*topfrp.fr_win).w_status_height;
            if ptr::eq(topfrp.fr_win, next_curwin) {
                m = opt_vars().p_wh as c_int + extra_height;
            } else {
                m = opt_vars().p_wmh as c_int + extra_height;
                if ptr::eq(topfrp.fr_win, curwin_ptr())
                    && next_curwin.is_null()
                    && opt_vars().p_wmh == 0
                {
                    m += 1;
                }
            }
        } else if topfrp.fr_layout == FR_ROW {
            m = 0;
            for frp in frame_iter_mut(topfrp.fr_child) {
                let n = frame_minheight(frp, next_curwin);
                if n > m {
                    m = n;
                }
            }
        } else {
            m = 0;
            for frp in frame_iter_mut(topfrp.fr_child) {
                m += frame_minheight(frp, next_curwin);
            }
        }
        m
    }
}

fn frame_minwidth(topfrp: &mut Frame, next_curwin: *mut WinInfoInfo) -> c_int {
    unsafe {
        let mut m: c_int;
        if !topfrp.fr_win.is_null() {
            if ptr::eq(topfrp.fr_win, next_curwin) {
                m = opt_vars().p_wiw as c_int + (*topfrp.fr_win).w_vsep_width;
            } else {
                m = opt_vars().p_wmw as c_int + (*topfrp.fr_win).w_vsep_width;
                if opt_vars().p_wmw == 0
                    && ptr::eq(topfrp.fr_win, curwin_ptr())
                    && next_curwin.is_null()
                {
                    m += 1;
                }
            }
        } else if topfrp.fr_layout == FR_COL {
            m = 0;
            for frp in frame_iter_mut(topfrp.fr_child) {
                let n = frame_minwidth(frp, next_curwin);
                m = m.max(n);
            }
        } else {
            m = 0;
            for frp in frame_iter_mut(topfrp.fr_child) {
                m += frame_minwidth(frp, next_curwin);
            }
        }
        m
    }
}

pub fn close_others(message: c_int, forceit: c_int, ignore_pinned: bool) {
    unsafe {
        let old_curwin = curwin_ptr();

        if curwin_mut().w_floating {
            if message != 0 && !g_st().autocmd_busy {
                emsg(E_FLOATONLY);
            }
            return;
        }

        if one_window(firstwin_mut(), ptr::null_mut()) && !lastwin_mut().w_floating {
            if message != 0 && !g_st().autocmd_busy {
                msg(gettext(M_ONLYONE), 0);
            }
            return;
        }

        let mut nextwp: *mut WinInfoInfo;
        let mut wp = g_st().firstwin;
        while win_valid(wp) {
            nextwp = (*wp).w_next;

            if !ptr::eq(old_curwin, curwin_ptr()) && win_valid(old_curwin) {
                set_curwin(old_curwin);
                g_st().curbuf = curwin_mut().w_buffer;
            }

            if ptr::eq(wp, curwin_ptr()) || ((*wp).w_onebuf_opt.wo_wp != 0 && !ignore_pinned) {
                wp = nextwp;
                continue;
            }

            if !buf_valid((*wp).w_buffer) && win_valid(wp) {
                (*wp).w_buffer = ptr::null_mut();
                win_close(wp, false, false);
                wp = nextwp;
                continue;
            }

            let r = can_abandon((*wp).buf_mut(), forceit != 0);
            if !win_valid(wp) {
                wp = g_st().firstwin;
                continue;
            }
            if !r {
                if message != 0
                    && (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
                    && opt_vars().p_write != 0
                {
                    dialog_changed((*wp).buf_mut(), false);
                    if !win_valid(wp) {
                        wp = g_st().firstwin;
                        continue;
                    }
                }
                if buf_is_changed_fn((*wp).buf_mut()) {
                    wp = nextwp;
                    continue;
                }
            }
            win_close(
                wp,
                !buf_hide((*wp).buf_mut()) && !buf_is_changed_fn((*wp).buf_mut()),
                false,
            );
            wp = nextwp;
        }

        if message != 0 && !one_window_macro() {
            let mut has_non_pinned = false;
            for wp2 in for_all_windows_ptr() {
                if !ptr::eq(wp2, curwin_ptr()) && (*wp2).w_onebuf_opt.wo_wp == 0 {
                    has_non_pinned = true;
                    break;
                }
            }
            if has_non_pinned {
                emsg(gettext(c"E445: Other window contains changes"));
            }
        }
    }
}

pub fn unuse_tabpage(tp: &mut TabPageInfo) {
    {
        tp.tp_topframe = g_st().topframe;
        tp.tp_firstwin = g_st().firstwin;
        tp.tp_lastwin = g_st().lastwin;
        tp.tp_curwin = curwin_ptr();
        tp.tp_ch_used = opt_vars().p_ch;
    }
}

pub fn use_tabpage(tp: *mut TabPageInfo) {
    {
        g_st().curtab = tp;
        g_st().topframe = curtab_mut().tp_topframe;
        g_st().firstwin = curtab_mut().tp_firstwin;
        g_st().lastwin = curtab_mut().tp_lastwin;
        set_curwin(curtab_mut().tp_curwin);
        opt_vars().p_ch = curtab_mut().tp_ch_used;
    }
}

pub fn win_alloc_first() {
    unsafe {
        if !win_alloc_firstwin(None) {
            std::process::abort();
        }

        g_st().first_tabpage = alloc_tabpage();
        g_st().curtab = g_st().first_tabpage;
        unuse_tabpage(&mut *g_st().first_tabpage);
    }
}

pub fn win_alloc_ctx_win(idx: c_int) {
    unsafe {
        let mut err = ERROR_INIT;
        let mut fconfig = WIN_CONFIG_INIT;
        fconfig.width = g_st().columns_global;
        fconfig.height = 5;
        fconfig.focusable = false;
        fconfig.mouse = false;
        fconfig.hide = true;
        let cw_win = win_new_float(ptr::null_mut(), true, fconfig, &mut err);
        g_st().ctx_win_vec.as_mut_slice()[idx as usize].cw_win = cw_win;
        (*cw_win).buf_mut().b_nwindows -= 1;
        reset_binding(&mut *cw_win);
    }
}

fn win_alloc_firstwin(oldwin: Option<&mut WinInfoInfo>) -> bool {
    unsafe {
        set_curwin(win_alloc(ptr::null_mut(), false));
        if let Some(oldwin) = oldwin {
            win_init(curwin_mut(), oldwin, 0);
            reset_binding(curwin_mut());
        } else {
            g_st().curbuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_LISTED);
            if g_st().curbuf.is_null() {
                return false;
            }
            curwin_mut().w_buffer = g_st().curbuf;
            curwin_mut().w_s_owned = None;
            curbuf_mut().b_nwindows = 1;
            curwin_mut().w_alist = None;
            curwin_init();
        }

        new_frame(curwin_mut());
        g_st().topframe = curwin_mut().w_frame;
        (*g_st().topframe).fr_width = g_st().columns_global;
        (*g_st().topframe).fr_height =
            g_st().rows_global - opt_vars().p_ch as c_int - global_stl_height();

        true
    }
}

fn new_frame(wp: &mut WinInfoInfo) {
    wp.w_frame = Box::into_raw(Box::new(Frame {
        fr_layout: FR_LEAF,
        fr_win: wp,
        ..Frame::new()
    }));
}

pub fn win_init_size() {
    unsafe {
        let avail = rows_avail() as c_int;
        firstwin_mut().w_height = avail;
        firstwin_mut().w_prev_height = avail;
        firstwin_mut().w_view_height = firstwin_mut().w_height - firstwin_mut().w_winbar_height;
        firstwin_mut().w_height_outer = firstwin_mut().w_height;
        firstwin_mut().w_winrow_off = firstwin_mut().w_winbar_height;
        (*g_st().topframe).fr_height = avail;
        firstwin_mut().w_width = g_st().columns_global;
        firstwin_mut().w_view_width = firstwin_mut().w_width;
        firstwin_mut().w_width_outer = firstwin_mut().w_width;
        (*g_st().topframe).fr_width = g_st().columns_global;
    }
}

fn alloc_tabpage() -> &'static mut TabPageInfo {
    unsafe {
        let tp = Box::leak(Box::new(TabPageInfo::new()));
        win_st().last_tp_handle += 1;
        tp.handle = win_st().last_tp_handle;
        g_st().tabpage_handles.insert(tp.handle, tp);

        init_var_dict(
            &mut *tv_dict_alloc(),
            &mut *(&raw mut tp.tp_winvar).cast(),
            VAR_SCOPE,
        );
        tp.tp_ch_used = opt_vars().p_ch;

        &mut *tp
    }
}

impl Drop for TabPageInfo {
    fn drop(&mut self) {
        {
            g_st().tabpage_handles.remove(&self.handle);
            diff_clear(self);
            for idx in 0..SNAP_COUNT {
                clear_snapshot(self, idx as c_int);
            }
            if !self.tp_winvar.di_tv.vval.v_dict().is_null() {
                vars_clear(&mut self.tp_vars().dv_hashtab);
                unref_var_dict(&mut *self.tp_vars());
            }
            if ptr::eq(self, g_st().lastused_tabpage) {
                g_st().lastused_tabpage = ptr::null_mut();
            }
        }
    }
}

pub fn free_tabpage(tp: *mut TabPageInfo) {
    unsafe {
        if !tp.is_null() {
            drop(Box::from_raw(tp));
        }
    }
}

pub fn win_new_tabpage(
    after: c_int,
    filename: impl crate::strings::IntoCCharPtr,
    enter: bool,
    first: *mut *mut WinInfoInfo,
) -> *mut TabPageInfo {
    let filename = filename.into_c_char_ptr();
    unsafe {
        let old_curtab = g_st().curtab;

        if window_layout_locked(Cmdidx::CmdTabnew) {
            return ptr::null_mut();
        }

        let newtp = alloc_tabpage();

        if enter {
            if !leave_tabpage(g_st().curbuf, true) {
                free_tabpage(newtp);
                return ptr::null_mut();
            }
        } else {
            unuse_tabpage(curtab_mut());
            curtab_mut().tp_old_rows_avail = rows_avail();
            g_st().firstwin = ptr::null_mut();
            g_st().lastwin = ptr::null_mut();
        }

        newtp.tp_localdir = (*old_curtab).tp_localdir.clone();

        g_st().curtab = newtp;

        let result = win_alloc_firstwin(((*old_curtab).tp_curwin).as_mut());
        debug_assert!(result);
        if !first.is_null() {
            *first = curwin_ptr();
        }

        if after == 1 {
            newtp.tp_next = g_st().first_tabpage;
            g_st().first_tabpage = newtp;
        } else {
            let mut tp = old_curtab;
            if after > 0 {
                let mut n: c_int = 2;
                tp = g_st().first_tabpage;
                while !(*tp).tp_next.is_null() && n < after {
                    n += 1;
                    tp = (*tp).tp_next;
                }
            }
            newtp.tp_next = (*tp).tp_next;
            (*tp).tp_next = newtp;
        }
        newtp.tp_firstwin = curwin_ptr();
        newtp.tp_lastwin = curwin_ptr();
        newtp.tp_curwin = curwin_ptr();

        win_init_size();
        firstwin_mut().w_winrow = tabline_height();
        firstwin_mut().w_prev_winrow = firstwin_mut().w_winrow;
        win_comp_scroll(curwin_mut());

        newtp.tp_topframe = g_st().topframe.cast();
        last_status(false);

        if !curbuf_mut().terminal.is_null() {
            terminal_check_size(&mut *curbuf_mut().terminal);
        }

        if enter {
            redraw_all_later(UPD_NOT_VALID);
            tabpage_check_windows(&mut *old_curtab);
            g_st().lastused_tabpage = old_curtab;
            entering_window(curwin_mut());

            apply_autocmds(
                EVENT_WINNEW,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            apply_autocmds(
                EVENT_WINENTER,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            apply_autocmds(EVENT_TABNEW, filename, filename, false, g_st().curbuf);
            apply_autocmds(
                EVENT_TABENTER,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        } else {
            unuse_tabpage(curtab_mut());
            use_tabpage(old_curtab);
            g_st().redraw_tabline = true;
            if curtab_mut().tp_old_rows_avail != rows_avail() {
                win_new_screen_rows();
            }

            let mut switchwin: CtxSwitch = CtxSwitch::new();
            let sw_ok = ctx_switch(
                &mut switchwin,
                newtp.tp_curwin,
                newtp,
                ptr::null_mut(),
                K_CTX_NO_DISPLAY,
            );
            debug_assert!(sw_ok);

            apply_autocmds(
                EVENT_WINNEW,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            apply_autocmds(EVENT_TABNEW, filename, filename, false, g_st().curbuf);

            ctx_restore(&mut switchwin);
        }

        newtp
    }
}

fn may_open_tabpage() -> bool {
    let n = if g_st().cmdmod.cmod_tab == 0 {
        g_st().postponed_split_tab
    } else {
        g_st().cmdmod.cmod_tab
    };

    if n == 0 {
        return false;
    }

    g_st().cmdmod.cmod_tab = 0;
    g_st().postponed_split_tab = 0;
    let status = !win_new_tabpage(n, ptr::null_mut(), true, ptr::null_mut()).is_null();
    if status {
        apply_autocmds(
            EVENT_TABNEWENTERED,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
    }
    status
}

pub fn make_tabpages(maxcount: c_int) -> c_int {
    unsafe {
        let count = maxcount.min(opt_vars().p_tpm as c_int);

        block_autocmds();

        let mut todo = count - 1;
        while todo > 0 {
            if win_new_tabpage(0, ptr::null_mut(), true, ptr::null_mut()).is_null() {
                break;
            }
            todo -= 1;
        }

        unblock_autocmds();

        count - todo
    }
}

pub fn valid_tabpage(tpc: *const TabPageInfo) -> bool {
    for tp in for_all_tabpages_ptr() {
        if ptr::eq(tp, tpc) {
            return true;
        }
    }
    false
}

pub fn valid_tabpage_win(tpc: &mut TabPageInfo) -> c_int {
    unsafe {
        for tp in for_all_tabpages_ptr() {
            if ptr::eq(tp, tpc) {
                for wp in win_iter_ptr(if ptr::eq(tp, g_st().curtab) {
                    g_st().firstwin
                } else {
                    (*tp).tp_firstwin
                }) {
                    if win_valid_any_tab(wp) {
                        return 1;
                    }
                }
                return 0;
            }
        }
        0
    }
}

pub fn close_tabpage(tab: &mut TabPageInfo) {
    unsafe {
        let mut ptp: *mut TabPageInfo;
        if ptr::eq(tab, g_st().first_tabpage) {
            g_st().first_tabpage = tab.tp_next;
            ptp = g_st().first_tabpage;
        } else {
            ptp = g_st().first_tabpage;
            while !ptp.is_null() && !ptr::eq((*ptp).tp_next, tab) {
                ptp = (*ptp).tp_next;
            }
            debug_assert!(!ptp.is_null());
            (*ptp).tp_next = tab.tp_next;
        }

        goto_tabpage_tp(ptp, false, false);
        free_tabpage(&raw mut *tab);
    }
}

pub fn find_tabpage(n: c_int) -> *mut TabPageInfo {
    unsafe {
        if n == 0 {
            return g_st().curtab;
        }

        let mut i: c_int = 1;
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() && i != n {
            i += 1;
            tp = (*tp).tp_next;
        }
        tp
    }
}

pub fn tabpage_index(ftp: *mut TabPageInfo) -> c_int {
    unsafe {
        let mut i: c_int = 1;
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() && !ptr::eq(tp, ftp) {
            i += 1;
            tp = (*tp).tp_next;
        }
        i
    }
}

fn leave_tabpage(new_curbuf: *mut FileBuf, trigger_leave_autocmds: bool) -> bool {
    unsafe {
        let tp = g_st().curtab;

        leaving_window(curwin_mut());
        reset_visual_and_resel();
        if trigger_leave_autocmds {
            if !ptr::eq(new_curbuf, g_st().curbuf) {
                apply_autocmds(
                    EVENT_BUFLEAVE,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                );
                if !ptr::eq(g_st().curtab, tp) {
                    return false;
                }
            }
            apply_autocmds(
                EVENT_WINLEAVE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            if !ptr::eq(g_st().curtab, tp) {
                return false;
            }
            apply_autocmds(
                EVENT_TABLEAVE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            if !ptr::eq(g_st().curtab, tp) {
                return false;
            }
        }

        reset_dragwin();
        (*tp).tp_curwin = curwin_ptr();
        (*tp).tp_prevwin = g_st().prevwin;
        (*tp).tp_firstwin = g_st().firstwin;
        (*tp).tp_lastwin = g_st().lastwin;
        (*tp).tp_old_rows_avail = rows_avail();
        if (*tp).tp_old_columns != -1 {
            (*tp).tp_old_columns = i64::from(g_st().columns_global);
        }
        g_st().firstwin = ptr::null_mut();
        g_st().lastwin = ptr::null_mut();
        true
    }
}

fn enter_tabpage(
    tp: &mut TabPageInfo,
    old_curbuf: &mut FileBuf,
    trigger_enter_autocmds: bool,
    trigger_leave_autocmds: bool,
) {
    unsafe {
        let old_off = (*tp.tp_firstwin).w_winrow;
        let next_prevwin = tp.tp_prevwin;
        let old_curtab = g_st().curtab;
        let prev_p_ch = opt_vars().p_ch;

        use_tabpage(&raw mut *tp);

        if !ptr::eq(old_curtab, g_st().curtab) && opt_vars().p_ch != prev_p_ch {
            tabpage_check_windows(&mut *old_curtab);
            let new_ch = opt_vars().p_ch;
            opt_vars().p_ch = prev_p_ch;
            win_st().command_frame_height = false;
            set_option_value(K_OPT_CMDHEIGHT, integer_obj(new_ch), 0, true, None);
            win_st().command_frame_height = true;
        } else if !ptr::eq(old_curtab, g_st().curtab) {
            tabpage_check_windows(&mut *old_curtab);
        }

        win_enter_ext(
            tp.tp_curwin,
            WEE_CURWIN_INVALID
                | (if trigger_enter_autocmds {
                    WEE_TRIGGER_ENTER_AUTOCMDS
                } else {
                    0
                })
                | (if trigger_leave_autocmds {
                    WEE_TRIGGER_LEAVE_AUTOCMDS
                } else {
                    0
                }),
        );
        g_st().prevwin = next_prevwin;

        last_status(false);
        win_float_update_statusline();
        win_comp_pos();
        g_st().diff_need_scrollbind = true;

        reset_dragwin();

        if curtab_mut().tp_old_rows_avail != rows_avail() || old_off != firstwin_mut().w_winrow {
            win_new_screen_rows();
        }
        if curtab_mut().tp_old_columns != i64::from(g_st().columns_global) {
            if starting_val() == 0 {
                win_new_screen_cols();
                curtab_mut().tp_old_columns = i64::from(g_st().columns_global);
            } else {
                curtab_mut().tp_old_columns = -1;
            }
        }

        g_st().lastused_tabpage = old_curtab;

        if trigger_enter_autocmds {
            apply_autocmds(
                EVENT_TABENTER,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            if !ptr::eq(old_curbuf, g_st().curbuf) {
                apply_autocmds(
                    EVENT_BUFENTER,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                );
            }
        }

        redraw_all_later(UPD_NOT_VALID);
    }
}

fn tabpage_check_windows(old_curtab: &mut TabPageInfo) {
    unsafe {
        for wp in win_iter_ptr(old_curtab.tp_firstwin) {
            if (*wp).w_floating {
                if (*wp).w_config.external {
                    win_remove(&mut *wp, &raw mut *old_curtab);
                    win_append(lastwin_nofloating(ptr::null_mut()), wp, ptr::null_mut());
                } else {
                    ui_comp_remove_grid(&mut (*wp).w_grid_alloc);
                }
            }
            (*wp).w_pos_changed = true;
        }

        for wp2 in for_all_windows_mut() {
            if wp2.w_floating && !wp2.w_config.external {
                win_config_float(wp2, wp2.w_config.clone());
            }
            wp2.w_pos_changed = true;
        }
    }
}

pub fn goto_tabpage(n: c_int) {
    unsafe {
        if text_locked() {
            text_locked_msg();
            return;
        }

        if (*g_st().first_tabpage).tp_next.is_null() {
            if n > 1 {
                beep_flush();
            }
            return;
        }

        let mut tp: *mut TabPageInfo = ptr::null_mut();

        if n == 0 {
            if curtab_mut().tp_next.is_null() {
                tp = g_st().first_tabpage;
            } else {
                tp = curtab_mut().tp_next;
            }
        } else if n < 0 {
            let mut ttp = g_st().curtab;
            for _ in n..0 {
                tp = g_st().first_tabpage;
                while !ptr::eq((*tp).tp_next, ttp) && !(*tp).tp_next.is_null() {
                    tp = (*tp).tp_next;
                }
                ttp = tp;
            }
        } else if n == 9999 {
            tp = g_st().first_tabpage;
            while !(*tp).tp_next.is_null() {
                tp = (*tp).tp_next;
            }
        } else {
            tp = find_tabpage(n);
            if tp.is_null() {
                beep_flush();
                return;
            }
        }

        goto_tabpage_tp(tp, true, true);
    }
}

pub fn goto_tabpage_tp(
    tp: *mut TabPageInfo,
    trigger_enter_autocmds: bool,
    trigger_leave_autocmds: bool,
) {
    unsafe {
        set_keep_msg(ptr::null(), 0);

        g_st().skip_win_fix_scroll = true;
        if !ptr::eq(tp, g_st().curtab)
            && leave_tabpage((*(*tp).tp_curwin).w_buffer, trigger_leave_autocmds)
        {
            if valid_tabpage(tp) {
                enter_tabpage(
                    &mut *tp,
                    crate::globals::curbuf_mut(),
                    trigger_enter_autocmds,
                    trigger_leave_autocmds,
                );
            } else {
                enter_tabpage(
                    curtab_mut(),
                    crate::globals::curbuf_mut(),
                    trigger_enter_autocmds,
                    trigger_leave_autocmds,
                );
            }
        }
        g_st().skip_win_fix_scroll = false;
    }
}

pub fn goto_tabpage_lastused() -> bool {
    if !valid_tabpage(g_st().lastused_tabpage) {
        return false;
    }

    goto_tabpage_tp(g_st().lastused_tabpage, true, true);
    true
}

pub fn goto_tabpage_win(tp: &mut TabPageInfo, wp: *mut WinInfoInfo) {
    goto_tabpage_tp(&raw mut *tp, true, true);
    if ptr::eq(g_st().curtab, tp) && win_valid(wp) {
        win_enter(wp, true);
    }
}

pub fn tabpage_move(nr: c_int) {
    unsafe {
        debug_assert!(!g_st().curtab.is_null());

        if (*g_st().first_tabpage).tp_next.is_null() {
            return;
        }

        if g_st().tabpage_move_disallowed != 0 {
            return;
        }

        let mut n: c_int = 1;
        let mut tp = g_st().first_tabpage;
        while !(*tp).tp_next.is_null() && n < nr {
            n += 1;
            tp = (*tp).tp_next;
        }

        if ptr::eq(tp, g_st().curtab)
            || (nr > 0 && !(*tp).tp_next.is_null() && ptr::eq((*tp).tp_next, g_st().curtab))
        {
            return;
        }

        let old_nr = tabpage_index(g_st().curtab);
        let tp_dst = tp;

        if ptr::eq(g_st().curtab, g_st().first_tabpage) {
            g_st().first_tabpage = curtab_mut().tp_next;
        } else {
            tp = ptr::null_mut();
            for tp2 in for_all_tabpages_ptr() {
                if ptr::eq((*tp2).tp_next, g_st().curtab) {
                    tp = tp2;
                    break;
                }
            }
            if tp.is_null() {
                return;
            }
            (*tp).tp_next = curtab_mut().tp_next;
        }

        if nr <= 0 {
            curtab_mut().tp_next = g_st().first_tabpage;
            g_st().first_tabpage = g_st().curtab;
        } else {
            curtab_mut().tp_next = (*tp_dst).tp_next;
            (*tp_dst).tp_next = g_st().curtab;
        }

        g_st().redraw_tabline = true;

        if has_event(EVENT_TABMOVED) {
            let mut prev_idx = [0 as c_char; NUMBUFLEN];
            vim_snprintf!(prev_idx.as_mut_ptr(), NUMBUFLEN, c"%i", old_nr);
            let mut data_items = [
                KeyValuePair {
                    key: cstr_as_string(c"tabnr_old"),
                    value: integer_obj(Integer::from(old_nr)),
                },
                KeyValuePair {
                    key: cstr_as_string(c"tabnr_new"),
                    value: integer_obj(Integer::from(tabpage_index(g_st().curtab))),
                },
            ];
            let data = Dict {
                items: data_items.as_mut_ptr(),
                size: 2,
                capacity: 2,
            };
            let mut obj = dict_obj(data);
            aucmd_defer(
                EVENT_TABMOVED,
                prev_idx.as_mut_ptr(),
                ptr::null_mut(),
                AUGROUP_ALL,
                curbuf_mut(),
                Some(&mut obj),
            );
        }
    }
}

pub fn win_goto(wp: *mut WinInfoInfo) {
    unsafe {
        let owp = curwin_ptr();

        if text_or_buf_locked() {
            beep_flush();
            return;
        }

        if !ptr::eq((*wp).w_buffer, g_st().curbuf) {
            reset_visual_and_resel();
        } else if g_st().visual_global.active {
            (*wp).w_cursor = curwin_mut().w_cursor;
        }

        if !win_valid(wp) {
            return;
        }

        win_enter(wp, true);

        if win_valid(owp) && (*owp).w_onebuf_opt.wo_cole > 0 && g_st().msg_scrolled == 0 {
            redraw_winline(&mut *owp, (*owp).w_cursor.lnum);
        }
        if curwin_mut().w_onebuf_opt.wo_cole > 0 && g_st().msg_scrolled == 0 {
            redraw_winline(crate::globals::curwin_mut(), curwin_mut().w_cursor.lnum);
        }
    }
}

pub fn win_find_tabpage(win: *const WinInfoInfo) -> *mut TabPageInfo {
    for tp in for_all_tabpages_mut() {
        for wp in for_all_windows_in_tab_ptr(tp) {
            if ptr::eq(wp, win) {
                return &raw mut *tp;
            }
        }
    }
    ptr::null_mut()
}

pub fn win_vert_neighbor(
    tp: &mut TabPageInfo,
    wp: &mut WinInfoInfo,
    up: bool,
    mut count: c_int,
) -> *mut WinInfoInfo {
    unsafe {
        let mut foundfr = wp.w_frame;

        if wp.w_floating {
            return if win_valid(g_st().prevwin) && !(*g_st().prevwin).w_floating {
                g_st().prevwin
            } else {
                g_st().firstwin
            };
        }

        'outer: while count > 0 {
            count -= 1;
            let mut nfr: *mut Frame;
            let mut fr = foundfr;
            loop {
                if ptr::eq(fr, tp.tp_topframe.cast()) {
                    break 'outer;
                }
                if up {
                    nfr = (*fr).fr_prev;
                } else {
                    nfr = (*fr).fr_next;
                }
                if (*(*fr).fr_parent).fr_layout == FR_COL && !nfr.is_null() {
                    break;
                }
                fr = (*fr).fr_parent;
            }

            loop {
                if (*nfr).fr_layout == FR_LEAF {
                    foundfr = nfr;
                    break;
                }
                fr = (*nfr).fr_child;
                if (*nfr).fr_layout == FR_ROW {
                    while !(*fr).fr_next.is_null()
                        && frame2win(fr).w_wincol + (*fr).fr_width <= wp.w_wincol + wp.w_wcol
                    {
                        fr = (*fr).fr_next;
                    }
                }
                if (*nfr).fr_layout == FR_COL && up {
                    while !(*fr).fr_next.is_null() {
                        fr = (*fr).fr_next;
                    }
                }
                nfr = fr;
            }
        }
        if !foundfr.is_null() {
            (*foundfr).fr_win
        } else {
            ptr::null_mut()
        }
    }
}

fn win_goto_ver(up: bool, count: c_int) {
    {
        let win = win_vert_neighbor(curtab_mut(), curwin_mut(), up, count);
        if !win.is_null() {
            win_goto(win);
        }
    }
}

pub fn win_horz_neighbor(
    tp: &mut TabPageInfo,
    wp: &mut WinInfoInfo,
    left: bool,
    mut count: c_int,
) -> *mut WinInfoInfo {
    unsafe {
        let mut foundfr = wp.w_frame;

        if wp.w_floating {
            return if win_valid(g_st().prevwin) && !(*g_st().prevwin).w_floating {
                g_st().prevwin
            } else {
                g_st().firstwin
            };
        }

        'outer: while count > 0 {
            count -= 1;
            let mut nfr: *mut Frame;
            let mut fr = foundfr;
            loop {
                if ptr::eq(fr, tp.tp_topframe.cast()) {
                    break 'outer;
                }
                if left {
                    nfr = (*fr).fr_prev;
                } else {
                    nfr = (*fr).fr_next;
                }
                if (*(*fr).fr_parent).fr_layout == FR_ROW && !nfr.is_null() {
                    break;
                }
                fr = (*fr).fr_parent;
            }

            loop {
                if (*nfr).fr_layout == FR_LEAF {
                    foundfr = nfr;
                    break;
                }
                fr = (*nfr).fr_child;
                if (*nfr).fr_layout == FR_COL {
                    while !(*fr).fr_next.is_null()
                        && frame2win(fr).w_winrow + (*fr).fr_height <= wp.w_winrow + wp.w_wrow
                    {
                        fr = (*fr).fr_next;
                    }
                }
                if (*nfr).fr_layout == FR_ROW && left {
                    while !(*fr).fr_next.is_null() {
                        fr = (*fr).fr_next;
                    }
                }
                nfr = fr;
            }
        }
        if !foundfr.is_null() {
            (*foundfr).fr_win
        } else {
            ptr::null_mut()
        }
    }
}

fn win_goto_hor(left: bool, count: c_int) {
    {
        let win = win_horz_neighbor(curtab_mut(), curwin_mut(), left, count);
        if !win.is_null() {
            win_goto(win);
        }
    }
}

pub fn win_enter(wp: *mut WinInfoInfo, undo_sync: bool) {
    win_enter_ext(
        wp,
        (if undo_sync { WEE_UNDO_SYNC } else { 0 })
            | WEE_TRIGGER_ENTER_AUTOCMDS
            | WEE_TRIGGER_LEAVE_AUTOCMDS,
    );
}

fn win_enter_ext(wp: *mut WinInfoInfo, flags: c_int) {
    unsafe {
        let mut other_buffer = false;
        let curwin_invalid = (flags & WEE_CURWIN_INVALID) != 0;

        if ptr::eq(wp, curwin_ptr()) && !curwin_invalid {
            return;
        }

        if !curwin_invalid {
            leaving_window(curwin_mut());
        }

        if !curwin_invalid && (flags & WEE_TRIGGER_LEAVE_AUTOCMDS) != 0 {
            if !ptr::eq((*wp).w_buffer, g_st().curbuf) {
                apply_autocmds(
                    EVENT_BUFLEAVE,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                );
                other_buffer = true;
                if !win_valid(wp) {
                    return;
                }
            }
            apply_autocmds(
                EVENT_WINLEAVE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            if !win_valid(wp) {
                return;
            }
            if aborting() {
                return;
            }
        }

        if (flags & WEE_UNDO_SYNC) != 0 && !ptr::eq(g_st().curbuf, (*wp).w_buffer) {
            u_sync(false);
        }

        if crate::memory::cstr_first_byte(opt_vars().p_spk) == b'c' && !curwin_invalid {
            update_topline(curwin_mut());
        }

        if !ptr::eq((*wp).w_buffer, g_st().curbuf) {
            buf_copy_options((*wp).buf_mut(), BCO_ENTER | BCO_NOHELP);
        }
        if !curwin_invalid {
            g_st().prevwin = curwin_ptr();
            curwin_mut().w_redr_status = true;
        }
        set_curwin(wp);
        g_st().curbuf = (*wp).w_buffer;

        check_cursor(curwin_mut());
        if !virtual_active(curwin_mut()) {
            curwin_mut().w_cursor.coladd = 0;
        }
        if crate::memory::cstr_first_byte(opt_vars().p_spk) == b'c' {
            changed_line_abv_curs();
        } else {
            win_fix_cursor((get_real_state() & (MODE_NORMAL | MODE_CMDLINE | MODE_TERMINAL)) != 0);
        }

        update_cwd(K_CD_CAUSE_WINDOW);

        entering_window(curwin_mut());
        if (flags & WEE_TRIGGER_NEW_AUTOCMDS) != 0 {
            apply_autocmds(
                EVENT_WINNEW,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }
        if (flags & WEE_TRIGGER_ENTER_AUTOCMDS) != 0 {
            apply_autocmds(
                EVENT_WINENTER,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
            if other_buffer {
                apply_autocmds(
                    EVENT_BUFENTER,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                );
            }
        }

        maketitle();
        curwin_mut().w_redr_status = true;
        g_st().redraw_tabline = true;
        if g_st().restart_edit != 0 {
            redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
        }

        if curwin_mut().w_hl_attr_normal != curwin_mut().w_hl_attr_normalnc {
            redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
        }
        if !g_st().prevwin.is_null()
            && (*g_st().prevwin).w_hl_attr_normal != (*g_st().prevwin).w_hl_attr_normalnc
        {
            redraw_later(g_st().prevwin.as_mut(), UPD_NOT_VALID);
        }

        if OptInt::from(curwin_mut().w_height) < opt_vars().p_wh
            && curwin_mut().w_onebuf_opt.wo_wfh == 0
            && !curwin_mut().w_floating
        {
            win_setheight(opt_vars().p_wh as c_int);
        } else if curwin_mut().w_height == 0 {
            win_setheight(1);
        }

        if OptInt::from(curwin_mut().w_width) < opt_vars().p_wiw
            && curwin_mut().w_onebuf_opt.wo_wfw == 0
            && !curwin_mut().w_floating
        {
            win_setwidth(opt_vars().p_wiw as c_int);
        }

        setmouse();
        do_autochdir();
    }
}

pub fn update_cwd(cause: CdCause) {
    {
        let new_dir: Option<std::ffi::CString>;
        let scope: CdScope;

        if let Some(ref d) = curbuf_mut().b_localdir {
            new_dir = Some(d.clone());
            scope = K_CD_SCOPE_BUFFER;
        } else if let Some(ref d) = curwin_mut().w_localdir {
            new_dir = Some(d.clone());
            scope = K_CD_SCOPE_WINDOW;
        } else if let Some(ref d) = curtab_mut().tp_localdir {
            new_dir = Some(d.clone());
            scope = K_CD_SCOPE_TABPAGE;
        } else {
            new_dir = None;
            scope = K_CD_SCOPE_GLOBAL;
        }

        let mut cwd = [0 as c_char; MAXPATHL];
        if !os_dirname(cwd.as_mut_ptr(), MAXPATHL) {
            cwd[0] = NUL;
        }

        if let Some(ref new_dir_cstr) = new_dir {
            let new_dir_ptr = new_dir_cstr.as_ptr().cast_mut();
            if g_st().globaldir.is_none() && cwd[0] != NUL {
                g_st().globaldir = Some(crate::memory::cstr_to_cstring(cwd.as_ptr()));
            }
            let dir_differs = !path_equal(new_dir_ptr, cwd.as_ptr(), K_PATH_CMP_LITERAL);
            if opt_vars().p_acd == 0 && dir_differs {
                do_autocmd_dirchanged(new_dir_ptr, scope, cause, true);
            }
            if os_chdir(new_dir_ptr) == 0 && opt_vars().p_acd == 0 && dir_differs {
                do_autocmd_dirchanged(new_dir_ptr, scope, cause, false);
            }
            g_st().last_chdir_reason = None;
            shorten_fnames(1);
        } else if let Some(gdir) = g_st().globaldir.take() {
            let gdir_ptr = gdir.as_ptr().cast_mut();
            let dir_differs = !path_equal(gdir_ptr, cwd.as_ptr(), K_PATH_CMP_LITERAL);
            if opt_vars().p_acd == 0 && dir_differs {
                do_autocmd_dirchanged(gdir_ptr, K_CD_SCOPE_GLOBAL, cause, true);
            }
            if os_chdir(gdir_ptr) == 0 && opt_vars().p_acd == 0 && dir_differs {
                do_autocmd_dirchanged(gdir_ptr, K_CD_SCOPE_GLOBAL, cause, false);
            }
            g_st().last_chdir_reason = None;
            shorten_fnames(1);
        }
    }
}

pub fn buf_jump_open_win(buf: *mut FileBuf) -> *mut WinInfoInfo {
    unsafe {
        if ptr::eq(curwin_mut().w_buffer, buf) {
            win_enter(curwin_ptr(), false);
            return curwin_ptr();
        }
        for wp in for_all_windows_ptr() {
            if ptr::eq((*wp).w_buffer, buf) {
                win_enter(wp, false);
                return wp;
            }
        }
        ptr::null_mut()
    }
}

pub fn buf_jump_open_tab(buf: *mut FileBuf) -> *mut WinInfoInfo {
    unsafe {
        let wp = buf_jump_open_win(buf);
        if !wp.is_null() {
            return wp;
        }

        for tp in for_all_tabpages_ptr() {
            if ptr::eq(tp, g_st().curtab) {
                continue;
            }
            let mut wp2 = (*tp).tp_firstwin;
            while !wp2.is_null() {
                if ptr::eq((*wp2).w_buffer, buf) {
                    goto_tabpage_win(&mut *tp, wp2);
                    if !ptr::eq(curwin_ptr(), wp2) {
                        wp2 = ptr::null_mut();
                    }
                    return wp2;
                }
                wp2 = (*wp2).w_next;
            }
        }

        ptr::null_mut()
    }
}

pub fn win_alloc(after: *mut WinInfoInfo, hidden: bool) -> &'static mut WinInfoInfo {
    unsafe {
        let new_wp = Box::leak(Box::new(WinInfoInfo::new()));

        win_st().last_win_id += 1;
        new_wp.handle = win_st().last_win_id;
        g_st().window_handles.insert(new_wp.handle, new_wp);

        grid_assign_handle(&mut new_wp.w_grid_alloc);

        init_var_dict(
            &mut *tv_dict_alloc(),
            &mut *(&raw mut new_wp.w_winvar).cast(),
            VAR_SCOPE,
        );

        block_autocmds();
        if !hidden {
            let mut tp: *mut TabPageInfo = ptr::null_mut();
            if !after.is_null() {
                tp = win_find_tabpage(after);
                if ptr::eq(tp, g_st().curtab) {
                    tp = ptr::null_mut();
                }
            }
            win_append(after, new_wp, tp);
        }

        new_wp.w_width = g_st().columns_global;

        new_wp.w_ns_hl = -1;

        new_wp.w_onebuf_opt.wo_so = -1;
        new_wp.w_allbuf_opt.wo_so = -1;
        new_wp.w_onebuf_opt.wo_sop = -1;
        new_wp.w_allbuf_opt.wo_sop = -1;
        new_wp.w_onebuf_opt.wo_siso = -1;
        new_wp.w_allbuf_opt.wo_siso = -1;

        new_wp.w_fraction = 0;
        new_wp.w_prev_fraction_row = -1;

        fold_init_win(&mut *new_wp);
        unblock_autocmds();
        new_wp.w_next_match_id = 1000;
        &mut *new_wp
    }
}

pub fn win_free(wp: *mut WinInfoInfo, tp: *mut TabPageInfo) {
    unsafe {
        g_st().window_handles.remove(&(*wp).handle);
        clear_folding(&mut *wp);

        (*wp).w_alist = None;

        block_autocmds();

        (*wp).w_ns_set.clear();

        clear_winopt(&mut (*wp).w_onebuf_opt);
        clear_winopt(&mut (*wp).w_allbuf_opt);

        (*wp).w_p_lcs_chars.multispace = Vec::new();
        (*wp).w_p_lcs_chars.leadmultispace = Vec::new();

        vars_clear(&mut (*wp).w_vars().dv_hashtab);
        unref_var_dict(&mut *(*wp).w_vars());

        if ptr::eq(g_st().prevwin, wp) {
            g_st().prevwin = ptr::null_mut();
        }
        for ttp in for_all_tabpages_mut() {
            if ptr::eq(ttp.tp_prevwin, wp) {
                ttp.tp_prevwin = ptr::null_mut();
            }
        }

        (*wp).w_lines.clear();

        for i in 0..(*wp).w_tagstacklen as usize {
            tagstack_clear_entry(&mut (*wp).w_tagstack[i]);
        }
        (*wp).w_tagstacklen = 0;

        (*wp).w_localdir = None;
        (*wp).w_prevdir = None;

        stl_clear_click_defs(&mut (*wp).w_status_click_defs);
        stl_clear_click_defs(&mut (*wp).w_winbar_click_defs);

        for (_, lnum_click_defs) in std::mem::take(&mut (*wp).w_statuscol_click_defs) {
            for (_, mut row_click_defs) in lnum_click_defs {
                stl_clear_click_defs(&mut row_click_defs);
            }
        }

        for buf in for_all_buffers_mut() {
            let mut pos_wip = None;
            let mut pos_null = None;
            for (i, wip) in buf.b_wininfo.iter().enumerate() {
                if ptr::eq(wip.wi_win, wp) {
                    pos_wip = Some(i);
                } else if wip.wi_win.is_null() {
                    pos_null = Some(i);
                }
            }

            if let Some(pw) = pos_wip {
                let wip_wp = &mut (&mut buf.b_wininfo)[pw];
                wip_wp.wi_win = ptr::null_mut();
                if (*wp).w_config.style == K_WIN_STYLE_MINIMAL && wip_wp.wi_optset {
                    clear_winopt(&mut wip_wp.wi_opt);
                    wip_wp.wi_folds.clear();
                    wip_wp.wi_optset = false;
                }
                if let Some(pn) = pos_null {
                    let pos_delete = pn.max(pw);
                    buf.b_wininfo.remove(pos_delete);
                }
            }
        }

        (*wp).w_config.title_chunks.clear();
        (*wp).w_config.footer_chunks.clear();

        clear_matches(&mut *wp);
        free_jumplist(&mut *wp);
        qf_free_all(wp.as_mut());

        (*wp).w_p_cc_cols.clear();

        win_free_grid(&mut *wp, false);

        if win_valid_any_tab(wp) {
            win_remove(&mut *wp, tp);
        }
        if g_st().autocmd_busy {
            g_st().au_pending_free_win.push(Box::from_raw(wp));
        } else {
            drop(Box::from_raw(wp));
        }

        unblock_autocmds();
    }
}

impl Drop for WinInfoInfo {
    fn drop(&mut self) {
        for i in 0..self.w_tagstacklen as usize {
            tagstack_clear_entry(&mut self.w_tagstack[i]);
        }
        self.w_tagstacklen = 0;

        self.w_localdir = None;
        self.w_prevdir = None;

        free_jumplist(self);
    }
}

pub fn win_free_grid(wp: &mut WinInfoInfo, reinit: bool) {
    {
        if wp.w_grid_alloc.handle != 0 && ui_has(K_UIMULTIGRID) {
            ui_call_grid_destroy(Integer::from(wp.w_grid_alloc.handle));
        }
        grid_free(&mut wp.w_grid_alloc);
        if reinit {
            wp.w_grid_alloc = SCREEN_GRID_INIT;
        }
    }
}

pub fn win_append(after: *mut WinInfoInfo, wp: *mut WinInfoInfo, tp: *mut TabPageInfo) {
    unsafe {
        debug_assert!(tp.is_null() || !ptr::eq(tp, g_st().curtab));

        let first = if tp.is_null() {
            &raw mut g_st().firstwin
        } else {
            &raw mut (*tp).tp_firstwin
        };
        let last = if tp.is_null() {
            &raw mut g_st().lastwin
        } else {
            &raw mut (*tp).tp_lastwin
        };

        let before = if after.is_null() {
            *first
        } else {
            (*after).w_next
        };

        (*wp).w_next = before;
        (*wp).w_prev = after;
        if after.is_null() {
            *first = wp;
        } else {
            (*after).w_next = wp;
        }
        if before.is_null() {
            *last = wp;
        } else {
            (*before).w_prev = wp;
        }
    }
}

pub fn win_remove(wp: &mut WinInfoInfo, tp: *mut TabPageInfo) {
    unsafe {
        debug_assert!(tp.is_null() || !ptr::eq(tp, g_st().curtab));

        if !wp.w_prev.is_null() {
            (*wp.w_prev).w_next = wp.w_next;
        } else if tp.is_null() {
            g_st().firstwin = wp.w_next;
            curtab_mut().tp_firstwin = wp.w_next;
        } else {
            (*tp).tp_firstwin = wp.w_next;
        }
        if !wp.w_next.is_null() {
            (*wp.w_next).w_prev = wp.w_prev;
        } else if tp.is_null() {
            g_st().lastwin = wp.w_prev;
            curtab_mut().tp_lastwin = wp.w_prev;
        } else {
            (*tp).tp_lastwin = wp.w_prev;
        }
    }
}

fn frame_append(after: &mut Frame, frp: &mut Frame) {
    unsafe {
        frp.fr_next = after.fr_next;
        after.fr_next = frp;
        if !frp.fr_next.is_null() {
            (*frp.fr_next).fr_prev = frp;
        }
        frp.fr_prev = after;
    }
}

fn frame_insert(before: &mut Frame, frp: &mut Frame) {
    unsafe {
        frp.fr_next = before;
        frp.fr_prev = before.fr_prev;
        before.fr_prev = frp;
        if !frp.fr_prev.is_null() {
            (*frp.fr_prev).fr_next = frp;
        } else {
            (*frp.fr_parent).fr_child = frp;
        }
    }
}

fn frame_remove(frp: &mut Frame) {
    unsafe {
        if !frp.fr_prev.is_null() {
            (*frp.fr_prev).fr_next = frp.fr_next;
        } else {
            (*frp.fr_parent).fr_child = frp.fr_next;
        }
        if !frp.fr_next.is_null() {
            (*frp.fr_next).fr_prev = frp.fr_prev;
        }
    }
}

pub fn win_new_screensize() {
    if win_st().old_rows != g_st().rows_global {
        if opt_vars().p_window == OptInt::from(win_st().old_rows - 1)
            || (win_st().old_rows == 0 && !option_was_set(K_OPT_WINDOW))
        {
            opt_vars().p_window = OptInt::from(g_st().rows_global - 1);
        }
        win_st().old_rows = g_st().rows_global;
        win_new_screen_rows();
    }
    if win_st().old_columns != g_st().columns_global {
        win_st().old_columns = g_st().columns_global;
        win_new_screen_cols();
    }
}

pub fn win_new_screen_rows() {
    unsafe {
        if g_st().firstwin.is_null() {
            return;
        }
        let h =
            (rows_avail() as c_int).max(frame_minheight(&mut *g_st().topframe, ptr::null_mut()));

        frame_new_height(g_st().topframe, h, false, true, false);
        if !frame_check_height(&*g_st().topframe, h) {
            frame_new_height(g_st().topframe, h, false, false, false);
        }

        win_comp_pos();
        win_reconfig_floats();
        compute_cmdrow();
        curtab_mut().tp_ch_used = opt_vars().p_ch;

        if !g_st().skip_win_fix_scroll {
            win_fix_scroll(true);
        }
    }
}

pub fn win_new_screen_cols() {
    unsafe {
        if g_st().firstwin.is_null() {
            return;
        }

        frame_new_width(&mut *g_st().topframe, g_st().columns_global, false, true);
        if !frame_check_width(&*g_st().topframe, g_st().columns_global) {
            frame_new_width(&mut *g_st().topframe, g_st().columns_global, false, false);
        }

        win_comp_pos();
        win_reconfig_floats();
    }
}

pub fn snapshot_windows_scroll_size() {
    {
        for wp in for_all_windows_mut() {
            wp.w_last_topline = wp.w_topline;
            wp.w_last_topfill = wp.w_topfill;
            wp.w_last_leftcol = wp.w_leftcol;
            wp.w_last_skipcol = wp.w_skipcol;
            wp.w_last_width = wp.w_width;
            wp.w_last_height = wp.w_height;
        }
    }
}

pub fn may_make_initial_scroll_size_snapshot() {
    if !win_st().did_initial_scroll_size_snapshot {
        win_st().did_initial_scroll_size_snapshot = true;
        snapshot_windows_scroll_size();
    }
}

fn make_win_info_dict(
    width: c_int,
    height: c_int,
    topline: c_int,
    topfill: c_int,
    leftcol: c_int,
    skipcol: c_int,
) -> *mut TvDict {
    {
        let d = tv_dict_alloc();
        d.dv_refcount = 1;

        let entries: [(&CStr, c_int); 6] = [
            (c"width", width),
            (c"height", height),
            (c"topline", topline),
            (c"topfill", topfill),
            (c"leftcol", leftcol),
            (c"skipcol", skipcol),
        ];

        for (key, val) in entries {
            let mut tv = TypVal {
                v_type: VarType::VarNumber,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(i64::from(val)),
            };
            if !tv_dict_add_tv(&mut *d, key.as_ptr(), key.to_bytes().len(), &mut tv) {
                tv_dict_unref(d);
                return ptr::null_mut();
            }
        }
        d
    }
}

fn check_window_scroll_resize(
    mut size_count: Option<&mut c_int>,
    first_scroll_win: *mut *mut WinInfoInfo,
    first_size_win: *mut *mut WinInfoInfo,
    mut winlist: Option<&mut TvList>,
    mut v_event: Option<&mut TvDict>,
) {
    unsafe {
        let mut tot_width: c_int = 0;
        let mut tot_height: c_int = 0;
        let mut tot_topline: c_int = 0;
        let mut tot_topfill: c_int = 0;
        let mut tot_leftcol: c_int = 0;
        let mut tot_skipcol: c_int = 0;

        for wp in for_all_windows_ptr() {
            if (*wp).w_floating && (*wp).w_last_topline == 0 {
                (*wp).w_last_topline = (*wp).w_topline;
                (*wp).w_last_topfill = (*wp).w_topfill;
                (*wp).w_last_leftcol = (*wp).w_leftcol;
                (*wp).w_last_skipcol = (*wp).w_skipcol;
                (*wp).w_last_width = (*wp).w_width;
                (*wp).w_last_height = (*wp).w_height;
                continue;
            }

            let ignore_scroll = event_ignored(EVENT_WINSCROLLED, (*wp).w_onebuf_opt.wo_eiw);
            let size_changed = !event_ignored(EVENT_WINRESIZED, (*wp).w_onebuf_opt.wo_eiw)
                && ((*wp).w_last_width != (*wp).w_width || (*wp).w_last_height != (*wp).w_height);
            if size_changed {
                if winlist.is_some() {
                    let tv = TypVal {
                        v_type: VarType::VarNumber,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::Number(i64::from((*wp).handle)),
                    };
                    tv_list_append_owned_tv(winlist.as_deref_mut().unwrap(), tv);
                } else if let Some(ref mut size_count) = size_count {
                    debug_assert!(!first_size_win.is_null() && !first_scroll_win.is_null());
                    **size_count += 1;
                    if (*first_size_win).is_null() {
                        *first_size_win = wp;
                    }
                    if (*first_scroll_win).is_null() && !ignore_scroll {
                        *first_scroll_win = wp;
                    }
                }
            }

            let scroll_changed = !ignore_scroll
                && ((*wp).w_last_topline != (*wp).w_topline
                    || (*wp).w_last_topfill != (*wp).w_topfill
                    || (*wp).w_last_leftcol != (*wp).w_leftcol
                    || (*wp).w_last_skipcol != (*wp).w_skipcol);
            if scroll_changed && !first_scroll_win.is_null() && (*first_scroll_win).is_null() {
                *first_scroll_win = wp;
            }

            if (size_changed || scroll_changed) && v_event.is_some() {
                let width = (*wp).w_width - (*wp).w_last_width;
                let height = (*wp).w_height - (*wp).w_last_height;
                let topline = (*wp).w_topline - (*wp).w_last_topline;
                let topfill = (*wp).w_topfill - (*wp).w_last_topfill;
                let leftcol = (*wp).w_leftcol - (*wp).w_last_leftcol;
                let skipcol = (*wp).w_skipcol - (*wp).w_last_skipcol;
                let d = make_win_info_dict(width, height, topline, topfill, leftcol, skipcol);
                if d.is_null() {
                    break;
                }
                let mut winid = [0 as c_char; NUMBUFLEN];
                let key_len = vim_snprintf!(winid.as_mut_ptr(), winid.len(), c"%d", (*wp).handle,);
                let _ = key_len;
                if !tv_dict_add_dict(v_event.as_deref_mut().unwrap(), winid.as_ptr(), d) {
                    tv_dict_unref(d);
                    break;
                }
                (*d).dv_refcount -= 1;

                tot_width += width.abs();
                tot_height += height.abs();
                tot_topline += topline.abs();
                tot_topfill += topfill.abs();
                tot_leftcol += leftcol.abs();
                tot_skipcol += skipcol.abs();
            }
        }

        if let Some(v_event) = v_event {
            let alldict = make_win_info_dict(
                tot_width,
                tot_height,
                tot_topline,
                tot_topfill,
                tot_leftcol,
                tot_skipcol,
            );
            if !alldict.is_null() {
                if !tv_dict_add_dict(v_event, c"all", alldict) {
                    tv_dict_unref(alldict);
                } else {
                    (*alldict).dv_refcount -= 1;
                }
            }
        }
    }
}

pub fn may_trigger_win_scrolled_resized() {
    unsafe {
        let do_resize = has_event(EVENT_WINRESIZED);
        let do_scroll = has_event(EVENT_WINSCROLLED);

        if win_st().win_scrolled_resized_recursive
            || !(do_scroll || do_resize)
            || !win_st().did_initial_scroll_size_snapshot
        {
            return;
        }

        let mut size_count: c_int = 0;
        let mut first_scroll_win: *mut WinInfoInfo = ptr::null_mut();
        let mut first_size_win: *mut WinInfoInfo = ptr::null_mut();
        check_window_scroll_resize(
            Some(&mut size_count),
            &raw mut first_scroll_win,
            &raw mut first_size_win,
            None,
            None,
        );
        let trigger_resize = do_resize && size_count > 0;
        let trigger_scroll = do_scroll && !first_scroll_win.is_null();
        if !trigger_resize && !trigger_scroll {
            return;
        }

        let mut windows_list: *mut TvList = ptr::null_mut();
        if trigger_resize {
            windows_list = tv_list_alloc(size_count as isize);
            check_window_scroll_resize(
                None,
                ptr::null_mut(),
                ptr::null_mut(),
                windows_list.as_mut(),
                None,
            );
        }

        let mut scroll_dict: *mut TvDict = ptr::null_mut();
        if trigger_scroll {
            scroll_dict = tv_dict_alloc();
            (*scroll_dict).dv_refcount = 1;
            check_window_scroll_resize(
                None,
                ptr::null_mut(),
                ptr::null_mut(),
                None,
                scroll_dict.as_mut(),
            );
        }

        snapshot_windows_scroll_size();

        win_st().win_scrolled_resized_recursive = true;

        let mut resize_winid = [0 as c_char; NUMBUFLEN];
        let mut resize_win: *mut WinInfoInfo = ptr::null_mut();
        let mut resize_bufref: Bufref = Bufref::new();
        if trigger_resize {
            resize_win = first_size_win;
            vim_snprintf!(
                resize_winid.as_mut_ptr(),
                resize_winid.len(),
                c"%d",
                (*resize_win).handle,
            );
            set_bufref(&mut resize_bufref, (*resize_win).w_buffer);
        }

        let mut scroll_winid = [0 as c_char; NUMBUFLEN];
        let mut scroll_win: *mut WinInfoInfo = ptr::null_mut();
        let mut scroll_bufref: Bufref = Bufref::new();
        if trigger_scroll {
            scroll_win = first_scroll_win;
            vim_snprintf!(
                scroll_winid.as_mut_ptr(),
                scroll_winid.len(),
                c"%d",
                (*scroll_win).handle,
            );
            set_bufref(&mut scroll_bufref, (*scroll_win).w_buffer);
        }

        if trigger_resize && !windows_list.is_null() {
            let mut save_v_event: SaveVEvent = SaveVEvent::new();
            let v_event = get_v_event(&mut save_v_event);

            if tv_dict_add_list(&mut *v_event, c"windows", windows_list) {
                tv_dict_set_keys_readonly(&mut *v_event);
                let buf = if bufref_valid(Some(&resize_bufref)) {
                    resize_bufref.br_buf
                } else {
                    g_st().curbuf
                };
                let win = if win_valid_any_tab(resize_win) {
                    resize_win
                } else {
                    curwin_ptr()
                };
                apply_autocmds_win(
                    EVENT_WINRESIZED,
                    resize_winid.as_mut_ptr(),
                    resize_winid.as_mut_ptr(),
                    false,
                    buf,
                    win.as_ref(),
                );
            }
            restore_v_event(&mut *v_event, &mut save_v_event);
        }

        if trigger_scroll && !scroll_dict.is_null() {
            let mut save_v_event: SaveVEvent = SaveVEvent::new();
            let v_event = get_v_event(&mut save_v_event);

            tv_dict_extend(&mut *v_event, &mut *scroll_dict, c"move");
            tv_dict_set_keys_readonly(&mut *v_event);
            tv_dict_unref(scroll_dict);

            let buf = if bufref_valid(Some(&scroll_bufref)) {
                scroll_bufref.br_buf
            } else {
                g_st().curbuf
            };
            let win = if win_valid_any_tab(scroll_win) {
                scroll_win
            } else {
                curwin_ptr()
            };
            apply_autocmds_win(
                EVENT_WINSCROLLED,
                scroll_winid.as_mut_ptr(),
                scroll_winid.as_mut_ptr(),
                false,
                buf,
                win.as_ref(),
            );

            restore_v_event(&mut *v_event, &mut save_v_event);
        }

        win_st().win_scrolled_resized_recursive = false;
    }
}

pub fn win_size_save(gap: &mut Vec<c_int>) {
    {
        gap.clear();
        gap.reserve((win_count() * 2 + 1) as usize);
        gap.push(rows_avail() as c_int + global_stl_height() - last_stl_height(false));

        for wp in for_all_windows_mut() {
            gap.push(wp.w_width + wp.w_vsep_width);
            gap.push(wp.w_height);
        }
    }
}

pub fn win_size_restore(gap: &[c_int]) {
    unsafe {
        if (win_count() * 2 + 1) as usize == gap.len()
            && gap[0] == rows_avail() as c_int + global_stl_height() - last_stl_height(false)
        {
            for _ in 0..2 {
                let mut i: usize = 1;
                for wp in for_all_windows_ptr() {
                    let width = gap[i];
                    let height = gap[i + 1];
                    i += 2;
                    if !(*wp).w_floating {
                        frame_setwidth(&mut *(*wp).w_frame, width, true);
                        win_setheight_win(height, &mut *wp, true);
                    }
                }
            }
            win_comp_pos();
        }
    }
}

pub fn win_comp_pos() -> c_int {
    unsafe {
        let mut row = tabline_height();
        let mut col: c_int = 0;

        frame_comp_pos(&mut *g_st().topframe, &mut row, &mut col);

        let mut wp = g_st().lastwin;
        while !wp.is_null() && (*wp).w_floating {
            if (*wp).w_config.relative == K_FLOAT_RELATIVE_WINDOW {
                (*wp).w_pos_changed = true;
            }
            wp = (*wp).w_prev;
        }

        row + global_stl_height()
    }
}

fn frame_comp_pos(topfrp: &mut Frame, row: &mut c_int, col: &mut c_int) {
    unsafe {
        let wp = topfrp.fr_win;
        if !wp.is_null() {
            if (*wp).w_winrow != *row || (*wp).w_wincol != *col {
                (*wp).w_winrow = *row;
                (*wp).w_wincol = *col;
                redraw_later(wp.as_mut(), UPD_NOT_VALID);
                (*wp).w_redr_status = true;
                (*wp).w_pos_changed = true;
            }
            let h = (*wp).w_height + (*wp).w_hsep_height + (*wp).w_status_height;
            *row += if h > topfrp.fr_height {
                topfrp.fr_height
            } else {
                h
            };
            *col += (*wp).w_width + (*wp).w_vsep_width;
        } else {
            let startrow = *row;
            let startcol = *col;
            for frp in frame_iter_mut(topfrp.fr_child) {
                if topfrp.fr_layout == FR_ROW {
                    *row = startrow;
                } else {
                    *col = startcol;
                }
                frame_comp_pos(frp, row, col);
            }
        }
    }
}

pub fn win_setheight(height: c_int) {
    {
        win_setheight_win(height, crate::globals::curwin_mut(), true);
    }
}

pub fn win_setheight_win(mut height: c_int, win: &mut WinInfoInfo, from_top: bool) {
    unsafe {
        let min_wmh = if ptr::eq(win, curwin_ptr()) {
            opt_vars().p_wmh.max(1)
        } else {
            opt_vars().p_wmh
        } as c_int;
        height = height.max(min_wmh + win.w_winbar_height);

        if win.w_floating {
            win.w_config.height = height.max(1);
            win_config_float(win, win.w_config.clone());
            redraw_later(Some(&mut *win), UPD_VALID);
        } else {
            frame_setheight(
                &mut *win.w_frame,
                height + win.w_hsep_height + win.w_status_height,
                from_top,
            );

            win_comp_pos();
            win_fix_scroll(true);

            redraw_all_later(UPD_NOT_VALID);
            g_st().redraw_cmdline = true;
        }
    }
}

fn frame_setheight(curfrp: &mut Frame, mut height: c_int, from_top: bool) {
    unsafe {
        if curfrp.fr_height == height {
            return;
        }

        if curfrp.fr_parent.is_null() {
            if height > 0 {
                frame_new_height(&raw mut *curfrp, height, false, false, true);
            }
        } else if (*curfrp.fr_parent).fr_layout == FR_ROW {
            let h = frame_minheight(&mut *curfrp.fr_parent, ptr::null_mut());
            height = height.max(h);
            frame_setheight(&mut *curfrp.fr_parent, height, from_top);
        } else {
            let mut room: c_int = 0;
            let mut room_cmdline: c_int = 0;
            let mut room_reserved: c_int = 0;

            for run in 1..=2 {
                room = 0;
                room_reserved = 0;
                for frp in frame_iter_ptr((*curfrp.fr_parent).fr_child) {
                    if !ptr::eq(frp, curfrp)
                        && !(*frp).fr_win.is_null()
                        && (*(*frp).fr_win).w_onebuf_opt.wo_wfh != 0
                    {
                        room_reserved += (*frp).fr_height;
                    }
                    room += (*frp).fr_height;
                    if !ptr::eq(frp, curfrp) {
                        room -= frame_minheight(&mut *frp, ptr::null_mut());
                    }
                }
                if !from_top || curfrp.fr_width != g_st().columns_global {
                    room_cmdline = 0;
                } else {
                    let wp = lastwin_nofloating(ptr::null_mut());
                    room_cmdline = g_st().rows_global
                        - opt_vars().p_ch as c_int
                        - global_stl_height()
                        - (wp.w_winrow + wp.w_height + wp.w_hsep_height + wp.w_status_height);
                    room_cmdline = room_cmdline.max(0);
                }

                if height <= room + room_cmdline {
                    break;
                }
                if run == 2 || curfrp.fr_width == g_st().columns_global {
                    height = room + room_cmdline;
                    break;
                }
                frame_setheight(
                    &mut *curfrp.fr_parent,
                    height + frame_minheight(&mut *curfrp.fr_parent, nowin())
                        - opt_vars().p_wmh as c_int
                        - 1,
                    from_top,
                );
            }

            let mut take = height - curfrp.fr_height;

            if height > room + room_cmdline - room_reserved {
                room_reserved = room + room_cmdline - height;
            }
            if take < 0 && room - curfrp.fr_height <= room_reserved {
                room_reserved = 0;
            }

            if take > 0 && room_cmdline > 0 {
                room_cmdline = room_cmdline.min(take);
                take -= room_cmdline;
                (*g_st().topframe).fr_height += room_cmdline;
            }

            frame_new_height(&raw mut *curfrp, height, false, false, true);

            for run in 0..2 {
                let forward = (run == 0) == from_top;
                let mut frp = if forward {
                    curfrp.fr_next
                } else {
                    curfrp.fr_prev
                };

                while !frp.is_null() && take != 0 {
                    let h = frame_minheight(&mut *frp, ptr::null_mut());
                    if room_reserved > 0
                        && !(*frp).fr_win.is_null()
                        && (*(*frp).fr_win).w_onebuf_opt.wo_wfh != 0
                    {
                        if room_reserved >= (*frp).fr_height {
                            room_reserved -= (*frp).fr_height;
                        } else {
                            if (*frp).fr_height - room_reserved > take {
                                room_reserved = (*frp).fr_height - take;
                            }
                            take -= (*frp).fr_height - room_reserved;
                            frame_new_height(frp, room_reserved, false, false, true);
                            room_reserved = 0;
                        }
                    } else if (*frp).fr_height - take < h {
                        take -= (*frp).fr_height - h;
                        frame_new_height(frp, h, false, false, true);
                    } else {
                        frame_new_height(frp, (*frp).fr_height - take, false, false, true);
                        take = 0;
                    }
                    frp = if forward {
                        (*frp).fr_next
                    } else {
                        (*frp).fr_prev
                    };
                }
            }
        }
    }
}

pub fn win_setwidth(width: c_int) {
    {
        win_setwidth_win(width, crate::globals::curwin_mut(), true);
    }
}

pub fn win_setwidth_win(mut width: c_int, wp: &mut WinInfoInfo, from_left: bool) {
    unsafe {
        if ptr::eq(wp, curwin_ptr()) {
            width = width.max(opt_vars().p_wmw as c_int).max(1);
        } else if width < 0 {
            width = 0;
        }
        if wp.w_floating {
            wp.w_config.width = width;
            win_config_float(wp, wp.w_config.clone());
            redraw_later(Some(&mut *wp), UPD_NOT_VALID);
        } else {
            frame_setwidth(&mut *wp.w_frame, width + wp.w_vsep_width, from_left);

            win_comp_pos();
            redraw_all_later(UPD_NOT_VALID);
        }
    }
}

fn frame_setwidth(curfrp: &mut Frame, mut width: c_int, from_left: bool) {
    unsafe {
        if curfrp.fr_width == width {
            return;
        }

        if curfrp.fr_parent.is_null() {
            return;
        }

        if (*curfrp.fr_parent).fr_layout == FR_COL {
            let w = frame_minwidth(&mut *curfrp.fr_parent, ptr::null_mut());
            width = width.max(w);
            frame_setwidth(&mut *curfrp.fr_parent, width, from_left);
        } else {
            let mut room: c_int = 0;
            let mut room_reserved: c_int = 0;
            for run in 1..=2 {
                room = 0;
                room_reserved = 0;
                for frp in frame_iter_ptr((*curfrp.fr_parent).fr_child) {
                    if !ptr::eq(frp, curfrp)
                        && !(*frp).fr_win.is_null()
                        && (*(*frp).fr_win).w_onebuf_opt.wo_wfw != 0
                    {
                        room_reserved += (*frp).fr_width;
                    }
                    room += (*frp).fr_width;
                    if !ptr::eq(frp, curfrp) {
                        room -= frame_minwidth(&mut *frp, ptr::null_mut());
                    }
                }

                if width <= room {
                    break;
                }
                if run == 2 || OptInt::from(curfrp.fr_height) >= rows_avail() {
                    width = room;
                    break;
                }
                frame_setwidth(
                    &mut *curfrp.fr_parent,
                    width + frame_minwidth(&mut *curfrp.fr_parent, nowin())
                        - opt_vars().p_wmw as c_int
                        - 1,
                    from_left,
                );
            }

            let mut take = width - curfrp.fr_width;

            if width > room - room_reserved {
                room_reserved = room - width;
            }
            if take < 0 && room - curfrp.fr_width < room_reserved {
                room_reserved = 0;
            }

            frame_new_width(curfrp, width, false, false);

            for run in 0..2 {
                let forward = (run == 0) == from_left;
                let mut frp = if forward {
                    curfrp.fr_next
                } else {
                    curfrp.fr_prev
                };

                while !frp.is_null() && take != 0 {
                    let w = frame_minwidth(&mut *frp, ptr::null_mut());
                    if room_reserved > 0
                        && !(*frp).fr_win.is_null()
                        && (*(*frp).fr_win).w_onebuf_opt.wo_wfw != 0
                    {
                        if room_reserved >= (*frp).fr_width {
                            room_reserved -= (*frp).fr_width;
                        } else {
                            if (*frp).fr_width - room_reserved > take {
                                room_reserved = (*frp).fr_width - take;
                            }
                            take -= (*frp).fr_width - room_reserved;
                            frame_new_width(&mut *frp, room_reserved, false, false);
                            room_reserved = 0;
                        }
                    } else if (*frp).fr_width - take < w {
                        take -= (*frp).fr_width - w;
                        frame_new_width(&mut *frp, w, false, false);
                    } else {
                        frame_new_width(&mut *frp, (*frp).fr_width - take, false, false);
                        take = 0;
                    }
                    frp = if forward {
                        (*frp).fr_next
                    } else {
                        (*frp).fr_prev
                    };
                }
            }
        }
    }
}

pub fn did_set_winminheight(_args: &mut Optset) -> *const c_char {
    let mut first = true;
    while opt_vars().p_wmh > 0 {
        let room = g_st().rows_global - opt_vars().p_ch as c_int;
        let needed = min_rows_for_all_tabpages();
        if room >= needed {
            break;
        }
        opt_vars().p_wmh -= 1;
        if first {
            emsg(gettext(E_NOROOM));
            first = false;
        }
    }
    ptr::null()
}

pub fn did_set_winminwidth(_args: &mut Optset) -> *const c_char {
    unsafe {
        let mut first = true;
        while opt_vars().p_wmw > 0 {
            let room = g_st().columns_global;
            let needed = frame_minwidth(&mut *g_st().topframe, ptr::null_mut());
            if room >= needed {
                break;
            }
            opt_vars().p_wmw -= 1;
            if first {
                emsg(gettext(E_NOROOM));
                first = false;
            }
        }
        ptr::null()
    }
}

pub fn win_drag_status_line(dragwin: &mut WinInfoInfo, mut offset: c_int) {
    unsafe {
        let mut fr = dragwin.w_frame;
        let mut curfr = fr;
        if !ptr::eq(fr, g_st().topframe) {
            fr = (*fr).fr_parent;
            if (*fr).fr_layout != FR_COL {
                curfr = fr;
                if !ptr::eq(fr, g_st().topframe) {
                    fr = (*fr).fr_parent;
                }
            }
        }

        while !ptr::eq(curfr, g_st().topframe) && (*curfr).fr_next.is_null() {
            if !ptr::eq(fr, g_st().topframe) {
                fr = (*fr).fr_parent;
            }
            curfr = fr;
            if !ptr::eq(fr, g_st().topframe) {
                fr = (*fr).fr_parent;
            }
        }

        let mut room: c_int;
        let up = offset < 0;

        if up {
            offset = -offset;
            if ptr::eq(fr, curfr) {
                room = (*fr).fr_height - frame_minheight(&mut *fr, ptr::null_mut());
            } else {
                room = 0;
                fr = (*fr).fr_child;
                loop {
                    room += (*fr).fr_height - frame_minheight(&mut *fr, ptr::null_mut());
                    if ptr::eq(fr, curfr) {
                        break;
                    }
                    fr = (*fr).fr_next;
                }
            }
            fr = (*curfr).fr_next;
        } else {
            room = g_st().rows_global - g_st().cmdline_row;
            if !(*curfr).fr_next.is_null() {
                room -= opt_vars().p_ch as c_int + global_stl_height();
            } else if win_st().min_set_ch > 0 {
                room -= 1;
            }
            room = room.max(0);
            fr = (*curfr).fr_next;
            while !fr.is_null() {
                room += (*fr).fr_height - frame_minheight(&mut *fr, ptr::null_mut());
                fr = (*fr).fr_next;
            }
            fr = curfr;
        }

        offset = offset.min(room);
        if offset <= 0 {
            return;
        }

        if !fr.is_null() {
            frame_new_height(fr, (*fr).fr_height + offset, up, false, true);
        }

        if up {
            fr = curfr;
        } else {
            fr = (*curfr).fr_next;
        }
        while !fr.is_null() && offset > 0 {
            let n = frame_minheight(&mut *fr, ptr::null_mut());
            if (*fr).fr_height - offset <= n {
                offset -= (*fr).fr_height - n;
                frame_new_height(fr, n, !up, false, true);
            } else {
                frame_new_height(fr, (*fr).fr_height - offset, !up, false, true);
                break;
            }
            if up {
                fr = (*fr).fr_prev;
            } else {
                fr = (*fr).fr_next;
            }
        }
        win_comp_pos();
        win_fix_scroll(true);

        redraw_all_later(UPD_SOME_VALID);
        showmode();
    }
}

pub fn win_drag_vsep_line(dragwin: &mut WinInfoInfo, mut offset: c_int) {
    unsafe {
        let mut fr = dragwin.w_frame;
        if ptr::eq(fr, g_st().topframe) {
            return;
        }
        let mut curfr = fr;
        fr = (*fr).fr_parent;
        if (*fr).fr_layout != FR_ROW {
            if ptr::eq(fr, g_st().topframe) {
                return;
            }
            curfr = fr;
            fr = (*fr).fr_parent;
        }

        while (*curfr).fr_next.is_null() {
            if ptr::eq(fr, g_st().topframe) {
                break;
            }
            curfr = fr;
            fr = (*fr).fr_parent;
            if !ptr::eq(fr, g_st().topframe) {
                curfr = fr;
                fr = (*fr).fr_parent;
            }
        }

        let mut room: c_int;
        let left = offset < 0;

        if left {
            offset = -offset;
            room = 0;
            fr = (*fr).fr_child;
            loop {
                room += (*fr).fr_width - frame_minwidth(&mut *fr, ptr::null_mut());
                if ptr::eq(fr, curfr) {
                    break;
                }
                fr = (*fr).fr_next;
            }
            fr = (*curfr).fr_next;
        } else {
            room = 0;
            fr = (*curfr).fr_next;
            while !fr.is_null() {
                room += (*fr).fr_width - frame_minwidth(&mut *fr, ptr::null_mut());
                fr = (*fr).fr_next;
            }
            fr = curfr;
        }

        offset = offset.min(room);
        if offset <= 0 || fr.is_null() {
            return;
        }

        frame_new_width(&mut *fr, (*fr).fr_width + offset, left, false);

        if left {
            fr = curfr;
        } else {
            fr = (*curfr).fr_next;
        }
        while !fr.is_null() && offset > 0 {
            let n = frame_minwidth(&mut *fr, ptr::null_mut());
            if (*fr).fr_width - offset <= n {
                offset -= (*fr).fr_width - n;
                frame_new_width(&mut *fr, n, !left, false);
            } else {
                frame_new_width(&mut *fr, (*fr).fr_width - offset, !left, false);
                break;
            }
            if left {
                fr = (*fr).fr_prev;
            } else {
                fr = (*fr).fr_next;
            }
        }
        win_comp_pos();
        redraw_all_later(UPD_NOT_VALID);
    }
}

pub fn set_fraction(wp: &mut WinInfoInfo) {
    {
        if wp.w_view_height > 1 {
            wp.w_fraction = (wp.w_wrow * FRACTION_MULT + FRACTION_MULT / 2) / wp.w_view_height;
        }
    }
}

pub fn win_fix_scroll(resize: bool) {
    unsafe {
        if crate::memory::cstr_first_byte(opt_vars().p_spk) == b'c' {
            return;
        }

        g_st().skip_update_topline = true;
        for wp in for_all_windows_ptr() {
            if !(*wp).w_floating && (*wp).w_height != (*wp).w_prev_height {
                (*wp).w_do_win_fix_cursor = true;

                if crate::memory::cstr_first_byte(opt_vars().p_spk) == b's'
                    && (*wp).w_winrow != (*wp).w_prev_winrow
                    && (*wp).w_botline - 1 <= (*wp).buf_mut().b_ml.ml_line_count
                {
                    let diff = ((*wp).w_winrow - (*wp).w_prev_winrow)
                        + ((*wp).w_height - (*wp).w_prev_height);
                    let cursor = (*wp).w_cursor;
                    (*wp).w_cursor.lnum = (*wp).w_botline - 1;

                    if diff > 0 {
                        cursor_down_inner(&mut *wp, diff, false);
                    } else {
                        cursor_up_inner(&mut *wp, -diff, false);
                    }

                    (*wp).w_fraction = FRACTION_MULT;
                    scroll_to_fraction(&mut *wp, (*wp).w_prev_height);
                    (*wp).w_cursor = cursor;
                    (*wp).w_valid &= !VALID_WCOL;
                } else if ptr::eq(wp, curwin_ptr()) {
                    (*wp).w_valid &= !VALID_CROW;
                }

                invalidate_botline_win(&mut *wp);
                validate_botline_win(&mut *wp);
            }
            (*wp).w_prev_height = (*wp).w_height;
            (*wp).w_prev_winrow = (*wp).w_winrow;
        }
        g_st().skip_update_topline = false;
        if (get_real_state() & (MODE_NORMAL | MODE_CMDLINE | MODE_TERMINAL)) == 0 {
            win_fix_cursor(false);
        } else if resize {
            win_fix_cursor(true);
        }
    }
}

fn win_fix_cursor(normal: bool) {
    unsafe {
        let wp = curwin_ptr();

        if !(*wp).w_do_win_fix_cursor || (*wp).buf_mut().b_ml.ml_line_count < (*wp).w_view_height {
            return;
        }

        (*wp).w_do_win_fix_cursor = false;
        let so =
            (OptInt::from((*wp).w_view_height / 2).min(get_scrolloff_value(&mut *wp))) as c_int;
        let lnum = (*wp).w_cursor.lnum;

        (*wp).w_cursor.lnum = (*wp).w_topline;
        cursor_down_inner(&mut *wp, so, false);
        let top = (*wp).w_cursor.lnum;

        (*wp).w_cursor.lnum = (*wp).w_botline - 1;
        cursor_up_inner(&mut *wp, so, false);
        let bot = (*wp).w_cursor.lnum;

        (*wp).w_cursor.lnum = lnum;
        let mut nlnum: Linenr = 0;
        if lnum > bot && ((*wp).w_botline - (*wp).buf_mut().b_ml.ml_line_count) != 1 {
            nlnum = bot;
        } else if lnum < top && (*wp).w_topline != 1 {
            nlnum = if so == (*wp).w_view_height / 2 {
                bot
            } else {
                top
            };
        }

        if nlnum != 0 {
            if normal {
                setmark(b'\'' as c_int);
                (*wp).w_cursor.lnum = nlnum;
            } else {
                (*wp).w_fraction = if nlnum == bot { FRACTION_MULT } else { 0 };
                scroll_to_fraction(&mut *wp, (*wp).w_prev_height);
                validate_botline_win(curwin_mut());
            }
        }
    }
}

pub fn win_new_height(wp: &mut WinInfoInfo, mut height: c_int) {
    {
        height = height.max(0);
        if wp.w_height == height {
            return;
        }

        wp.w_height = height;
        wp.w_pos_changed = true;
        win_set_inner_size(wp, true);
    }
}

pub fn scroll_to_fraction(wp: &mut WinInfoInfo, prev_height: c_int) {
    unsafe {
        let height = wp.w_view_height;

        if height > 0
            && (wp.w_onebuf_opt.wo_scb == 0 || ptr::eq(wp, curwin_ptr()))
            && (height < wp.buf_mut().b_ml.ml_line_count || wp.w_topline > 1)
        {
            let mut lnum = wp.w_cursor.lnum.max(1);
            wp.w_wrow = (wp.w_fraction * height - 1) / FRACTION_MULT;
            let mut line_size = plines_win_col(wp, lnum, wp.w_cursor.col as std::ffi::c_long) - 1;
            let mut sline = wp.w_wrow - line_size;

            if sline >= 0 {
                let rows = plines_win(wp, lnum, false);
                if sline > wp.w_view_height - rows {
                    sline = wp.w_view_height - rows;
                    wp.w_wrow -= rows - line_size;
                }
            }

            if sline < 0 {
                wp.w_wrow = line_size;
                if wp.w_wrow >= wp.w_view_height && (wp.w_view_width - win_col_off(wp)) > 0 {
                    let mut skipcol: Colnr = wp.w_view_width - win_col_off(wp);
                    wp.w_wrow -= 1;
                    while wp.w_wrow >= wp.w_view_height {
                        skipcol += wp.w_view_width - win_col_off(wp) + win_col_off2(&mut *wp);
                        wp.w_wrow -= 1;
                    }
                    wp.w_skipcol = skipcol;
                }
            } else if sline > 0 {
                while sline > 0 && lnum > 1 {
                    has_folding(wp, lnum, Some(&mut lnum), ptr::null_mut());
                    if lnum == 1 {
                        line_size = c_int::from(!decor_conceal_line(wp, lnum - 1, false));
                        sline -= 1;
                        break;
                    }
                    lnum -= 1;
                    if lnum == wp.w_topline {
                        line_size = plines_win_nofill(wp, lnum, true) + wp.w_topfill;
                    } else {
                        line_size = plines_win(wp, lnum, true);
                    }
                    sline -= line_size;
                }

                if sline < 0 {
                    has_folding(wp, lnum, None, &raw mut lnum);
                    lnum += 1;
                    wp.w_wrow -= line_size + sline;
                } else if sline > 0 {
                    lnum = 1;
                    wp.w_wrow -= sline;
                }
            }
            set_topline(wp, lnum);
        }

        if ptr::eq(wp, curwin_ptr()) {
            curs_columns(&mut *wp, 0);
        }
        if prev_height > 0 {
            wp.w_prev_fraction_row = wp.w_wrow;
        }

        redraw_later(Some(&mut *wp), UPD_SOME_VALID);
        invalidate_botline_win(wp);
    }
}

pub fn win_set_inner_size(wp: &mut WinInfoInfo, valid_cursor: bool) {
    unsafe {
        let mut width = wp.w_width_request;
        if width == 0 {
            width = wp.w_width;
        }

        let prev_height = wp.w_view_height;
        let mut height = wp.w_height_request;
        if height == 0 {
            height = (wp.w_height - wp.w_winbar_height).max(0);
        }

        if height != prev_height {
            if height > 0 && valid_cursor {
                if ptr::eq(wp, curwin_ptr())
                    && (crate::memory::cstr_first_byte(opt_vars().p_spk) == b'c' || wp.w_floating)
                {
                    validate_cursor(curwin_mut());
                }
                if wp.w_view_height != prev_height {
                    return;
                }
                if wp.w_wrow != wp.w_prev_fraction_row {
                    set_fraction(&mut *wp);
                }
            }
            wp.w_view_height = height;
            win_comp_scroll(wp);

            if valid_cursor
                && !g_st().exiting
                && (crate::memory::cstr_first_byte(opt_vars().p_spk) == b'c' || wp.w_floating)
            {
                if wp.w_onebuf_opt.wo_sms == 0 {
                    wp.w_skipcol = 0;
                }
                scroll_to_fraction(wp, prev_height);
            }
            redraw_later(Some(&mut *wp), UPD_SOME_VALID);
        }

        if width != wp.w_view_width {
            wp.w_view_width = width;
            wp.w_lines_valid = 0;
            if valid_cursor {
                changed_line_abv_curs_win(wp);
                invalidate_botline_win(wp);
                if ptr::eq(wp, curwin_ptr())
                    && (crate::memory::cstr_first_byte(opt_vars().p_spk) == b'c' || wp.w_floating)
                {
                    curs_columns(&mut *wp, 1);
                }
            }
            redraw_later(Some(&mut *wp), UPD_NOT_VALID);
        }

        if !wp.buf_mut().terminal.is_null() {
            terminal_check_size(&mut *wp.buf_mut().terminal);
        }

        let float_stl_height = if wp.w_floating && wp.w_status_height != 0 {
            STATUS_HEIGHT
        } else {
            0
        };
        wp.w_height_outer =
            wp.w_view_height + win_border_height(wp) + wp.w_winbar_height + float_stl_height;
        wp.w_width_outer = wp.w_view_width + win_border_width(wp);
        wp.w_winrow_off = wp.w_border_adj[0] + wp.w_winbar_height;
        wp.w_wincol_off = wp.w_border_adj[3];

        if ui_has(K_UIMULTIGRID) {
            ui_call_win_viewport_margins(
                Integer::from(wp.w_grid_alloc.handle),
                wp.handle,
                Integer::from(wp.w_winrow_off),
                Integer::from(wp.w_border_adj[2]),
                Integer::from(wp.w_wincol_off),
                Integer::from(wp.w_border_adj[1]),
            );
        }

        wp.w_redr_status = true;

        if g_st().updating_screen {
            win_grid_alloc(wp);
        }
    }
}

pub fn win_new_width(wp: &mut WinInfoInfo, width: c_int) {
    wp.w_width = if width < 0 { 0 } else { width };
    wp.w_pos_changed = true;
    win_set_inner_size(&mut *wp, true);
}

pub fn win_default_scroll(wp: &mut WinInfoInfo) -> OptInt {
    OptInt::from(wp.w_view_height / 2).max(1)
}

pub fn win_comp_scroll(wp: &mut WinInfoInfo) {
    let old_w_p_scr = wp.w_onebuf_opt.wo_scr;
    wp.w_onebuf_opt.wo_scr = win_default_scroll(wp);

    if wp.w_onebuf_opt.wo_scr != old_w_p_scr {
        wp.w_onebuf_opt.wo_script_ctx[K_WIN_OPT_SCROLL].sc_sid = SID_WINLAYOUT;
        wp.w_onebuf_opt.wo_script_ctx[K_WIN_OPT_SCROLL].sc_lnum = 0;
    }
}

pub fn command_height() {
    unsafe {
        let mut old_p_ch = curtab_mut().tp_ch_used as c_int;

        let mut frp = lastwin_nofloating(ptr::null_mut()).w_frame;
        while (*frp).fr_width != g_st().columns_global && !(*frp).fr_parent.is_null() {
            frp = (*frp).fr_parent;
        }

        while !(*frp).fr_prev.is_null()
            && (*frp).fr_layout == FR_LEAF
            && (*(*frp).fr_win).w_onebuf_opt.wo_wfh != 0
        {
            frp = (*frp).fr_prev;
        }

        while opt_vars().p_ch > OptInt::from(old_p_ch) && win_st().command_frame_height {
            if frp.is_null() {
                emsg(gettext(E_NOROOM));
                opt_vars().p_ch = OptInt::from(old_p_ch);
                break;
            }
            let h = ((opt_vars().p_ch - OptInt::from(old_p_ch)) as c_int)
                .min((*frp).fr_height - frame_minheight(&mut *frp, ptr::null_mut()));
            frame_add_height(frp, -h);
            old_p_ch += h;
            frp = (*frp).fr_prev;
        }
        if opt_vars().p_ch < OptInt::from(old_p_ch)
            && win_st().command_frame_height
            && !frp.is_null()
        {
            frame_add_height(frp, (OptInt::from(old_p_ch) - opt_vars().p_ch) as c_int);
        }

        win_comp_pos();
        win_fix_scroll(true);
        g_st().cmdline_row = g_st().rows_global - opt_vars().p_ch as c_int;
        g_st().redraw_cmdline = true;

        if g_st().msg_scrolled == 0 && g_st().full_screen {
            let mut grid: *mut GridView = &raw mut g_st().default_gridview;
            if !ui_has(K_UIMESSAGES) {
                msg_grid_validate();
                grid = &raw mut g_st().msg_grid_adj;
            }
            grid_clear(
                grid,
                g_st().cmdline_row,
                g_st().rows_global,
                0,
                g_st().columns_global,
                0,
            );
            g_st().msg_row = g_st().cmdline_row;
        }

        curtab_mut().tp_ch_used = opt_vars().p_ch;
        win_st().min_set_ch = opt_vars().p_ch;
    }
}

fn frame_add_height(mut frp: *mut Frame, n: c_int) {
    unsafe {
        frame_new_height(frp, (*frp).fr_height + n, false, false, false);
        loop {
            frp = (*frp).fr_parent;
            if frp.is_null() {
                break;
            }
            (*frp).fr_height += n;
        }
    }
}

pub fn last_status(morewin: bool) {
    unsafe {
        last_status_rec(
            &mut *g_st().topframe,
            last_stl_height(morewin) > 0,
            global_stl_height() > 0,
        );
        win_float_anchor_laststatus();
    }
}

pub fn win_remove_status_line(wp: &mut WinInfoInfo, add_hsep: bool) {
    wp.w_status_height = 0;
    if add_hsep {
        wp.w_hsep_height = 1;
    } else {
        let new_h = (if wp.w_floating {
            wp.w_view_height
        } else {
            wp.w_height
        }) + STATUS_HEIGHT;
        win_new_height(wp, new_h);
    }
    comp_col();

    stl_clear_click_defs(&mut wp.w_status_click_defs);
    wp.w_status_click_defs = Vec::new();
}

fn find_horizontally_resizable_frame(fr: &mut Frame) -> *mut Frame {
    unsafe {
        let mut fp = fr;
        while fp.fr_height <= frame_minheight(fp, ptr::null_mut()) {
            if ptr::eq(fp, g_st().topframe) {
                return ptr::null_mut();
            }
            if (*fp.fr_parent).fr_layout == FR_COL && !fp.fr_prev.is_null() {
                fp = &mut *fp.fr_prev;
            } else {
                fp = &mut *fp.fr_parent;
            }
        }
        fp
    }
}

fn resize_frame_for_status(fr: &mut Frame) -> bool {
    unsafe {
        let wp = fr.fr_win;
        let fp = find_horizontally_resizable_frame(fr);

        if fp.is_null() {
            emsg(gettext(E_NOROOM));
            false
        } else if !ptr::eq(fp, fr) {
            let old_height = fr.fr_height;
            frame_setheight(fr, old_height + 1, false);
            if fr.fr_height == old_height {
                emsg(gettext(E_NOROOM));
                return false;
            }
            win_comp_pos();
            true
        } else {
            win_new_height(&mut *wp, (*wp).w_height - 1);
            true
        }
    }
}

fn resize_frame_for_winbar(fr: &mut Frame) -> bool {
    let old_height = fr.fr_height;
    frame_setheight(fr, old_height + 1, false);
    if fr.fr_height == old_height {
        emsg(gettext(E_NOROOM));
        return false;
    }
    win_comp_pos();
    true
}

fn last_status_rec(fr: &mut Frame, statusline: bool, is_stl_global: bool) {
    unsafe {
        if fr.fr_layout == FR_LEAF {
            let wp = fr.fr_win;
            let is_last = is_bottom_win(&mut *wp);

            if is_last {
                if (*wp).w_status_height != 0 && (!statusline || is_stl_global) {
                    win_remove_status_line(&mut *wp, false);
                } else if (*wp).w_status_height == 0 && !is_stl_global && statusline {
                    (*wp).w_status_height = STATUS_HEIGHT;
                    if !resize_frame_for_status(fr) {
                        return;
                    }
                    comp_col();
                }
                if ((*wp).w_height - (*wp).w_prev_height).abs() == 1 {
                    (*wp).w_prev_height = (*wp).w_height;
                }
            } else if (*wp).w_status_height != 0 && is_stl_global {
                win_remove_status_line(&mut *wp, true);
            } else if (*wp).w_status_height == 0 && !is_stl_global {
                (*wp).w_status_height = STATUS_HEIGHT;
                (*wp).w_hsep_height = 0;
                comp_col();
            }
        } else {
            for fp in frame_iter_mut(fr.fr_child) {
                last_status_rec(fp, statusline, is_stl_global);
            }
        }
    }
}

pub fn set_winbar_win(wp: &mut WinInfoInfo, make_room: bool, valid_cursor: bool) -> c_int {
    unsafe {
        let winbar_height = if wp.w_floating {
            c_int::from(!crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_wbr))
        } else {
            c_int::from(
                !crate::memory::cstr_is_empty(opt_vars().p_wbr)
                    || !crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_wbr),
            )
        };

        if wp.w_winbar_height != winbar_height {
            if winbar_height == 1 && wp.w_view_height <= 1 {
                if wp.w_floating {
                    emsg(gettext(E_NOROOM));
                    return NOTDONE;
                } else if !make_room || !resize_frame_for_winbar(&mut *wp.w_frame) {
                    return FAIL;
                }
            }
            wp.w_winbar_height = winbar_height;
            win_set_inner_size(wp, valid_cursor);

            if winbar_height == 0 {
                stl_clear_click_defs(&mut wp.w_winbar_click_defs);
                wp.w_winbar_click_defs = Vec::new();
            }
        }

        OK
    }
}

fn set_winbar_curtab(make_room: bool, valid_cursor: bool) -> bool {
    {
        for wp in for_all_windows_ptr() {
            if set_winbar_win(unsafe { &mut *wp }, make_room, valid_cursor) == FAIL {
                return false;
            }
        }
        true
    }
}

pub fn set_winbar(make_room: bool) {
    set_winbar_curtab(make_room, true);
}

pub fn set_winbar_all(make_room: bool) {
    unsafe {
        let _ = set_winbar_curtab(make_room, true);

        for tp in for_all_tabpages_ptr() {
            if !ptr::eq(tp, g_st().curtab) {
                let mut switchwin: CtxSwitch = CtxSwitch::new();
                if ctx_switch(
                    &mut switchwin,
                    (*tp).tp_curwin,
                    tp,
                    ptr::null_mut(),
                    K_CTX_NO_EVENTS | K_CTX_NO_DISPLAY,
                ) {
                    let _ = set_winbar_curtab(make_room, false);
                }
                ctx_restore(&mut switchwin);
            }
        }
    }
}

pub fn tabline_height() -> c_int {
    unsafe {
        if ui_has(K_UITABLINE) {
            return 0;
        }
        debug_assert!(!g_st().first_tabpage.is_null());
        match opt_vars().p_stal {
            0 => 0,
            1 => c_int::from(!(*g_st().first_tabpage).tp_next.is_null()),
            _ => 1,
        }
    }
}

pub fn global_winbar_height() -> c_int {
    c_int::from(!crate::memory::cstr_is_empty(opt_vars().p_wbr))
}

pub fn global_stl_height() -> c_int {
    if opt_vars().p_ls == 3 {
        STATUS_HEIGHT
    } else {
        0
    }
}

pub fn last_stl_height(morewin: bool) -> c_int {
    if opt_vars().p_ls > 1
        || (opt_vars().p_ls == 1 && (morewin || !one_window(firstwin_mut(), ptr::null_mut())))
    {
        STATUS_HEIGHT
    } else {
        0
    }
}

pub fn min_rows(tp: &mut TabPageInfo) -> c_int {
    unsafe {
        if g_st().firstwin.is_null() {
            return MIN_LINES;
        }

        let mut total = frame_minheight(&mut *tp.tp_topframe, ptr::null_mut());
        total += tabline_height() + global_stl_height();
        if (if ptr::eq(tp, g_st().curtab) {
            opt_vars().p_ch
        } else {
            tp.tp_ch_used
        }) > 0
        {
            total += 1;
        }
        total
    }
}

pub fn min_rows_for_all_tabpages() -> c_int {
    unsafe {
        if g_st().firstwin.is_null() {
            return MIN_LINES;
        }

        let mut total: c_int = 0;
        for tp in for_all_tabpages_ptr() {
            let mut n = frame_minheight(&mut *(*tp).tp_topframe, ptr::null_mut());
            if (if ptr::eq(tp, g_st().curtab) {
                opt_vars().p_ch
            } else {
                (*tp).tp_ch_used
            }) > 0
            {
                n += 1;
            }
            total = total.max(n);
        }
        total += tabline_height() + global_stl_height();
        total
    }
}

pub fn only_one_window() -> bool {
    unsafe {
        if !(*g_st().first_tabpage).tp_next.is_null() {
            return false;
        }

        let mut count: c_int = 0;
        for wp in for_all_windows_ptr() {
            if !(*wp).w_buffer.is_null()
                && (!((bt_help(((*wp).w_buffer).as_ref())
                    && !bt_help(Some(crate::globals::curbuf_mut())))
                    || (*wp).w_floating
                    || (*wp).w_onebuf_opt.wo_pvw != 0)
                    || ptr::eq(wp, curwin_ptr()))
                && !is_ctx_win(wp)
            {
                count += 1;
            }
        }
        count <= 1
    }
}

fn check_lnums_both(do_curwin: bool, nested: bool) {
    unsafe {
        for tp in for_all_tabpages_ptr() {
            for wp in win_iter_ptr(if ptr::eq(tp, g_st().curtab) {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            }) {
                if (do_curwin || !ptr::eq(wp, curwin_ptr()))
                    && ptr::eq((*wp).w_buffer, g_st().curbuf)
                {
                    if !nested {
                        (*wp).w_save_cursor.w_cursor_save = (*wp).w_cursor;
                        (*wp).w_save_cursor.w_topline_save = (*wp).w_topline;
                    }

                    let mut need_adjust = (*wp).w_cursor.lnum > curbuf_mut().b_ml.ml_line_count;
                    if need_adjust {
                        (*wp).w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
                    }
                    if need_adjust || !nested {
                        (*wp).w_save_cursor.w_cursor_corr = (*wp).w_cursor;
                    }

                    need_adjust = (*wp).w_topline > curbuf_mut().b_ml.ml_line_count;
                    if need_adjust {
                        (*wp).w_topline = curbuf_mut().b_ml.ml_line_count;
                    }
                    if need_adjust || !nested {
                        (*wp).w_save_cursor.w_topline_corr = (*wp).w_topline;
                    }
                }
            }
        }
    }
}

pub fn check_lnums(do_curwin: bool) {
    check_lnums_both(do_curwin, false);
}

pub fn check_lnums_nested(do_curwin: bool) {
    check_lnums_both(do_curwin, true);
}

pub fn reset_lnums() {
    unsafe {
        for tp in for_all_tabpages_ptr() {
            for wp in win_iter_mut(if ptr::eq(tp, g_st().curtab) {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            }) {
                if ptr::eq(wp.w_buffer, g_st().curbuf) {
                    if equalpos(wp.w_save_cursor.w_cursor_corr, wp.w_cursor)
                        && wp.w_save_cursor.w_cursor_save.lnum != 0
                    {
                        wp.w_cursor = wp.w_save_cursor.w_cursor_save;
                    }
                    if wp.w_save_cursor.w_topline_corr == wp.w_topline
                        && wp.w_save_cursor.w_topline_save != 0
                    {
                        wp.w_topline = wp.w_save_cursor.w_topline_save;
                    }
                    if wp.w_save_cursor.w_topline_save > wp.buf_mut().b_ml.ml_line_count {
                        wp.w_valid &= !VALID_TOPLINE;
                    }
                }
            }
        }
    }
}

pub fn make_snapshot(idx: c_int) {
    unsafe {
        curtab_mut().tp_snapshot[idx as usize] = Some(make_snapshot_rec(&mut *g_st().topframe));
    }
}

fn make_snapshot_rec(fr: &mut Frame) -> Box<SnapshotFrame> {
    unsafe {
        Box::new(SnapshotFrame {
            fr_layout: fr.fr_layout,
            fr_width: fr.fr_width,
            fr_height: fr.fr_height,
            fr_next: (!fr.fr_next.is_null()).then(|| make_snapshot_rec(&mut *fr.fr_next)),
            fr_child: (!fr.fr_child.is_null()).then(|| make_snapshot_rec(&mut *fr.fr_child)),
            fr_win: if fr.fr_layout == FR_LEAF && ptr::eq(fr.fr_win, curwin_ptr()) {
                curwin_ptr()
            } else {
                ptr::null_mut()
            },
        })
    }
}

fn clear_snapshot(tp: &mut TabPageInfo, idx: c_int) {
    tp.tp_snapshot[idx as usize] = None;
}

fn get_snapshot_curwin_rec(ft: &SnapshotFrame) -> *mut WinInfoInfo {
    if let Some(ref next) = ft.fr_next {
        let wp = get_snapshot_curwin_rec(next);
        if !wp.is_null() {
            return wp;
        }
    }
    if let Some(ref child) = ft.fr_child {
        let wp = get_snapshot_curwin_rec(child);
        if !wp.is_null() {
            return wp;
        }
    }
    ft.fr_win
}

fn get_snapshot_curwin(idx: c_int) -> *mut WinInfoInfo {
    {
        let Some(ref sn) = curtab_mut().tp_snapshot[idx as usize] else {
            return ptr::null_mut();
        };
        get_snapshot_curwin_rec(sn)
    }
}

pub fn restore_snapshot(idx: c_int, close_curwin: c_int) {
    unsafe {
        if let Some(sn) = curtab_mut().tp_snapshot[idx as usize].take()
            && sn.fr_width == (*g_st().topframe).fr_width
            && sn.fr_height == (*g_st().topframe).fr_height
            && check_snapshot_rec(&sn, &mut *g_st().topframe)
        {
            let wp = restore_snapshot_rec(&sn, &mut *g_st().topframe);
            win_comp_pos();
            if !wp.is_null() && close_curwin != 0 {
                win_goto(wp);
            }
            redraw_all_later(UPD_NOT_VALID);
        }
    }
}

fn check_snapshot_rec(sn: &SnapshotFrame, fr: &mut Frame) -> bool {
    unsafe {
        !(sn.fr_layout != fr.fr_layout
            || sn.fr_next.is_none() != fr.fr_next.is_null()
            || sn.fr_child.is_none() != fr.fr_child.is_null()
            || (sn
                .fr_next
                .as_deref()
                .is_some_and(|next| !check_snapshot_rec(next, &mut *fr.fr_next)))
            || (sn
                .fr_child
                .as_deref()
                .is_some_and(|child| !check_snapshot_rec(child, &mut *fr.fr_child)))
            || (!sn.fr_win.is_null() && !win_valid(sn.fr_win)))
    }
}

fn restore_snapshot_rec(sn: &SnapshotFrame, fr: &mut Frame) -> *mut WinInfoInfo {
    unsafe {
        let mut wp: *mut WinInfoInfo = ptr::null_mut();

        fr.fr_height = sn.fr_height;
        fr.fr_width = sn.fr_width;
        if fr.fr_layout == FR_LEAF {
            frame_new_height(&raw mut *fr, fr.fr_height, false, false, false);
            frame_new_width(fr, fr.fr_width, false, false);
            wp = sn.fr_win;
        }
        if let Some(ref next) = sn.fr_next {
            let wp2 = restore_snapshot_rec(next, &mut *fr.fr_next);
            if !wp2.is_null() {
                wp = wp2;
            }
        }
        if let Some(ref child) = sn.fr_child {
            let wp2 = restore_snapshot_rec(child, &mut *fr.fr_child);
            if !wp2.is_null() {
                wp = wp2;
            }
        }
        wp
    }
}

fn frame_check_height(topfrp: &Frame, height: c_int) -> bool {
    {
        if topfrp.fr_height != height {
            return false;
        }
        if topfrp.fr_layout == FR_ROW {
            for frp in frame_iter_mut(topfrp.fr_child) {
                if frp.fr_height != height {
                    return false;
                }
            }
        }
        true
    }
}

fn frame_check_width(topfrp: &Frame, width: c_int) -> bool {
    {
        if topfrp.fr_width != width {
            return false;
        }
        if topfrp.fr_layout == FR_COL {
            for frp in frame_iter_mut(topfrp.fr_child) {
                if frp.fr_width != width {
                    return false;
                }
            }
        }
        true
    }
}

pub fn check_colorcolumn(cc: *mut c_char, mut wp: Option<&mut WinInfoInfo>) -> *const c_char {
    unsafe {
        if wp.is_some() && wp.as_deref_mut().unwrap().w_buffer.is_null() {
            return ptr::null();
        }

        let mut s: *mut c_char = EMPTY_STRING_OPTION.as_ptr().cast::<c_char>();
        if !cc.is_null() {
            s = cc;
        } else if wp.is_some() {
            s = wp.as_deref_mut().unwrap().w_onebuf_opt.wo_cc;
        }

        let tw: OptInt = if wp.is_some() {
            wp.as_deref_mut().unwrap().buf_mut().b_p_tw
        } else {
            0
        };

        let mut count: usize = 0;
        let mut color_cols = [0 as c_int; 256];
        while !crate::memory::cstr_is_empty(s) && count < 255 {
            let mut col: c_int;
            let mut skip = false;
            if crate::memory::cstr_first_byte(s) == b'-'
                || crate::memory::cstr_first_byte(s) == b'+'
            {
                col = if crate::memory::cstr_first_byte(s) == b'-' {
                    -1
                } else {
                    1
                };
                s = s.add(1);
                if !ascii_isdigit(c_int::from(crate::memory::cstr_first_byte(s))) {
                    return E_INVARG.as_ptr();
                }
                col *= getdigits_int(&raw mut s, true, 0);
                if tw == 0 {
                    skip = true;
                } else {
                    col += tw as c_int;
                    if col < 0 {
                        skip = true;
                    }
                }
            } else if ascii_isdigit(c_int::from(crate::memory::cstr_first_byte(s))) {
                col = getdigits_int(&raw mut s, true, 0);
            } else {
                return E_INVARG.as_ptr();
            }
            if !skip {
                color_cols[count] = col - 1;
                count += 1;
            }
            if crate::memory::cstr_is_empty(s) {
                break;
            }
            if crate::memory::cstr_first_byte(s) != b',' {
                return E_INVARG.as_ptr();
            }
            s = s.add(1);
            if crate::memory::cstr_is_empty(s) {
                return E_INVARG.as_ptr();
            }
        }

        if wp.is_none() {
            return ptr::null();
        }

        wp.as_deref_mut().unwrap().w_p_cc_cols.clear();
        if count > 0 {
            color_cols[..count].sort_unstable();
            wp.as_deref_mut()
                .unwrap()
                .w_p_cc_cols
                .extend_from_slice(&color_cols[..count]);
            wp.as_deref_mut().unwrap().w_p_cc_cols.dedup();
            wp.unwrap().w_p_cc_cols.push(-1);
        }

        ptr::null()
    }
}

pub fn win_locked(wp: &mut WinInfoInfo) -> c_int {
    wp.w_locked
}

pub fn win_get_tabwin(id: ObjHandle, tabnr: &mut c_int, winnr: &mut c_int) {
    {
        *tabnr = 0;
        *winnr = 0;

        for (tnum, tp) in (1 as c_int..).zip(for_all_tabpages_ptr()) {
            let mut wnum: c_int = 1;
            for wp in for_all_windows_in_tab_mut(unsafe { &*tp }) {
                if wp.handle == id {
                    if win_has_winnr(wp, unsafe { &mut *tp }) {
                        *winnr = wnum;
                        *tabnr = tnum;
                    }
                    return;
                }
                wnum += c_int::from(win_has_winnr(wp, unsafe { &mut *tp }));
            }
        }
    }
}

pub fn win_ui_flush(validate: bool) {
    unsafe {
        for tp in for_all_tabpages_ptr() {
            for wp in win_iter_mut(if ptr::eq(tp, g_st().curtab) {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            }) {
                if (wp.w_pos_changed || wp.w_grid_alloc.pending_comp_index_update)
                    && !wp.w_grid_alloc.chars.is_empty()
                {
                    if ptr::eq(tp, g_st().curtab) {
                        ui_ext_win_position(wp, validate);
                    } else {
                        ui_call_win_hide(Integer::from(wp.w_grid_alloc.handle));
                        wp.w_pos_changed = false;
                    }
                    wp.w_grid_alloc.pending_comp_index_update = false;
                }
                if ptr::eq(tp, g_st().curtab) {
                    ui_ext_win_viewport(wp);
                }
            }
        }
        pum_pos_ui_flush(false);
        msg_ui_flush();
    }
}

pub fn lastwin_nofloating(tp: *mut TabPageInfo) -> &'static mut WinInfoInfo {
    unsafe {
        debug_assert!(!ptr::eq(tp, g_st().curtab) || tp.is_null());
        let mut res = if !tp.is_null() {
            (*tp).tp_lastwin
        } else {
            g_st().lastwin
        };
        while (*res).w_floating {
            res = (*res).w_prev;
        }
        &mut *res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equalpos() {
        let p1 = CursorPos {
            lnum: 5,
            col: 10,
            coladd: 0,
        };
        let p2 = CursorPos {
            lnum: 5,
            col: 10,
            coladd: 0,
        };
        let p3 = CursorPos {
            lnum: 5,
            col: 11,
            coladd: 0,
        };
        assert!(equalpos(p1, p2));
        assert!(!equalpos(p1, p3));
    }

    #[test]
    fn test_window_layout_lock_unlock_and_frames_locked() {
        assert!(!frames_locked());
        window_layout_lock();
        assert!(win_st().split_disallowed > 0);
        assert!(win_st().close_disallowed > 0);
        window_layout_unlock();
        assert!(win_st().split_disallowed == 0);
        assert!(win_st().close_disallowed == 0);
    }

    #[test]
    fn test_frame_list_helpers() {
        let mut parent: Frame = Frame::new();
        let mut f1: Frame = Frame::new();
        let mut f2: Frame = Frame::new();
        let mut f3: Frame = Frame::new();
        f1.fr_parent = &raw mut parent;
        f2.fr_parent = &raw mut parent;
        f3.fr_parent = &raw mut parent;
        parent.fr_child = &raw mut f1;

        frame_append(&mut f1, &mut f2);
        assert!(ptr::eq(f1.fr_next, &raw mut f2));
        assert!(ptr::eq(f2.fr_prev, &raw mut f1));

        frame_insert(&mut f1, &mut f3);
        assert!(ptr::eq(parent.fr_child, &raw mut f3));
        assert!(ptr::eq(f3.fr_next, &raw mut f1));
        assert!(ptr::eq(f1.fr_prev, &raw mut f3));

        frame_remove(&mut f1);
        assert!(ptr::eq(f3.fr_next, &raw mut f2));
        assert!(ptr::eq(f2.fr_prev, &raw mut f3));
    }
}
