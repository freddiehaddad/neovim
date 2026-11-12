//! Window and tabpage layout, management, and navigation (`src/nvim/window.c`).
#![forbid(unsafe_code)]

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
    MAXLNUM, VALID_TOPLINE, VAR_SCOPE, bt_help, bt_prompt, bt_quickfix, buf_hide, buf_valid,
    buflist_findname_exp, buflist_findnr, buflist_getfile, buflist_new, bufref_valid, close_buffer,
    do_autochdir, do_buffer, maketitle, set_bufref,
};
use crate::charset::getdigits_int;
use crate::cmdexpand::{CTRL_N, CTRL_P, Cmdidx};
use crate::context::{CdCause, CtxSwitch, K_CD_CAUSE_WINDOW, ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::{Colnr, CursorPos, Linenr, check_cursor, check_cursor_lnum, equalpos};
use crate::decoration::decor_conceal_line;
use crate::diff::{diff_clear, diffopt_closeoff};
use crate::drawscreen::{
    FR_LEAF, FR_ROW, UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, comp_col, conceal_cursor_line,
    conceal_redraw_cursor_line, redraw_all_later, redraw_later, showmode, status_redraw_all,
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
    GridTarget, GridView, ScreenGrid, grid_adjust, grid_assign_handle, grid_clear, grid_free,
    win_grid_alloc,
};
use crate::input::{beep_flush, langmap_adjust, plain_vgetc};
use crate::insert::{
    CTRL_J_CONST, CTRL_S_CONST, CTRL_X_CONST, beginline, cursor_down_inner, cursor_up_inner,
};
use crate::keycodes::{CAR, K_BS, K_DOWN, K_KENTER, K_LEFT, K_RIGHT, K_UP, TAB};
use crate::main_init::getout;
use crate::mark::{copy_jumplist, free_jumplist, setmark, setpcmark};
use crate::r#match::clear_matches;
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
use crate::strings::{format_to_buf, gettext};
use crate::syntax::reset_synblock;
use crate::tag::{K_OPT_SWB_FLAG_USEOPEN, tagstack_clear_entry};
use crate::terminal::terminal_check_size;
use crate::ui::{
    ui_call_grid_destroy, ui_call_win_close, ui_call_win_external_pos, ui_call_win_float_pos,
    ui_call_win_hide, ui_call_win_pos, ui_call_win_viewport, ui_call_win_viewport_margins,
    ui_check_cursor_grid, ui_has,
};
use crate::ui_compositor::{
    ui_comp_compose_grid, ui_comp_layers_adjust, ui_comp_put_grid, ui_comp_remove_grid,
};
use crate::undo::{buf_is_changed_fn, u_sync};
use crate::winfloat::{
    win_border_height, win_border_width, win_config_float, win_float_anchor_laststatus,
    win_float_find_altwin, win_float_update_statusline, win_new_float, win_reconfig_floats,
};
use std::ffi::CStr;
use std::ptr;

pub type WinKind = i32;
pub(crate) const VALID_BOTLINE_AP: i32 = 0x40;

pub(crate) const K_WIN_OPT_COUNT: usize = 53;

#[repr(C)]
#[derive(Clone)]
pub struct PosSave {
    pub w_topline_save: std::cell::Cell<i32>,
    pub w_topline_corr: std::cell::Cell<i32>,
    pub w_cursor_save: std::cell::Cell<CursorPos>,
    pub w_cursor_corr: std::cell::Cell<CursorPos>,
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

pub use crate::option_vars::Winopt;

#[repr(C)]
#[derive(Clone)]
pub struct WinConfig {
    pub window: crate::api::tabpage::Window,
    pub bufpos: crate::cursor::Lpos,
    pub height: i32,
    pub width: i32,
    pub row: f64,
    pub col: f64,
    pub anchor: i32,
    pub relative: i32,
    pub external: bool,
    pub focusable: bool,
    pub mouse: bool,
    pub split: i32,
    pub zindex: i32,
    pub style: i32,
    pub border: bool,
    pub shadow: bool,
    pub border_chars: [[u8; 32]; 8],
    pub border_hl_ids: [i32; 8],
    pub border_attr: [i32; 8],
    pub title: bool,
    pub title_pos: i32,
    pub title_chunks: crate::decoration::VirtText,
    pub title_width: i32,
    pub footer: bool,
    pub footer_pos: i32,
    pub footer_chunks: crate::decoration::VirtText,
    pub footer_width: i32,
    pub noautocmd: bool,
    pub fixed: bool,
    pub hide: bool,
    pub _cmdline_offset: i32,
}

#[repr(C)]
#[derive(Debug)]
pub struct Frame {
    pub self_weak: std::rc::Weak<Self>,
    pub fr_layout: std::cell::Cell<u8>,
    pub fr_width: std::cell::Cell<i32>,
    pub fr_newwidth: std::cell::Cell<i32>,
    pub fr_height: std::cell::Cell<i32>,
    pub fr_newheight: std::cell::Cell<i32>,
    pub fr_parent: crate::api::private::validate::CloneCell<crate::globals::FramePtr>,
    pub fr_next: crate::api::private::validate::CloneCell<crate::globals::FramePtr>,
    pub fr_prev: crate::api::private::validate::CloneCell<crate::globals::FramePtr>,
    pub fr_child: crate::api::private::validate::CloneCell<crate::globals::FramePtr>,
    pub fr_win: crate::globals::CloneCell<crate::globals::WinPtr>,
}

impl Frame {
    pub fn new_rc() -> std::rc::Rc<Self> {
        std::rc::Rc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            fr_layout: std::cell::Cell::new(0),
            fr_width: std::cell::Cell::new(0),
            fr_newwidth: std::cell::Cell::new(0),
            fr_height: std::cell::Cell::new(0),
            fr_newheight: std::cell::Cell::new(0),
            fr_parent: crate::api::private::validate::CloneCell::new(
                crate::globals::FramePtr::NULL,
            ),
            fr_next: crate::api::private::validate::CloneCell::new(crate::globals::FramePtr::NULL),
            fr_prev: crate::api::private::validate::CloneCell::new(crate::globals::FramePtr::NULL),
            fr_child: crate::api::private::validate::CloneCell::new(crate::globals::FramePtr::NULL),
            fr_win: crate::globals::CloneCell::new(crate::globals::WinPtr::NULL),
        })
    }

    pub fn new_rc_from(other: &Self) -> std::rc::Rc<Self> {
        std::rc::Rc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            fr_layout: std::cell::Cell::new(other.fr_layout.get()),
            fr_width: std::cell::Cell::new(other.fr_width.get()),
            fr_newwidth: std::cell::Cell::new(other.fr_newwidth.get()),
            fr_height: std::cell::Cell::new(other.fr_height.get()),
            fr_newheight: std::cell::Cell::new(other.fr_newheight.get()),
            fr_parent: crate::api::private::validate::CloneCell::new(other.fr_parent.get()),
            fr_next: crate::api::private::validate::CloneCell::new(other.fr_next.get()),
            fr_prev: crate::api::private::validate::CloneCell::new(other.fr_prev.get()),
            fr_child: crate::api::private::validate::CloneCell::new(other.fr_child.get()),
            fr_win: crate::globals::CloneCell::new(other.fr_win.get()),
        })
    }

    #[inline]
    pub fn clear_links(&self) {
        self.fr_parent.set(crate::globals::FramePtr::NULL);
        self.fr_next.set(crate::globals::FramePtr::NULL);
        self.fr_prev.set(crate::globals::FramePtr::NULL);
        self.fr_child.set(crate::globals::FramePtr::NULL);
        self.fr_win.set(crate::globals::WinPtr::NULL);
    }
}

#[repr(C)]
pub struct WinInfoInfo {
    pub self_weak: std::rc::Weak<Self>,
    pub handle: ObjHandle,
    pub w_buffer: crate::globals::CloneCell<crate::globals::BufPtr>,
    pub w_s_owned: crate::globals::CloneCell<Option<std::rc::Rc<crate::syntax::Synblock>>>,
    pub w_ns_hl: std::cell::Cell<i32>,
    pub w_ns_hl_winhl: std::cell::Cell<i32>,
    pub w_ns_hl_active: std::cell::Cell<i32>,
    pub w_ns_hl_attr: crate::globals::CloneCell<Option<std::rc::Rc<crate::highlight::NSHlAttr>>>,
    pub w_ns_set: std::cell::RefCell<std::collections::BTreeSet<u32>>,
    pub w_hl_id_normal: std::cell::Cell<i32>,
    pub w_hl_attr_normal: std::cell::Cell<i32>,
    pub w_hl_attr_normalnc: std::cell::Cell<i32>,
    pub w_hl_needs_update: std::cell::Cell<i32>,
    pub w_prev: crate::globals::CloneCell<crate::globals::WinPtr>,
    pub w_next: crate::globals::CloneCell<crate::globals::WinPtr>,
    pub w_locked: std::cell::Cell<i32>,
    pub w_frame: crate::globals::CloneCell<crate::globals::FramePtr>,
    pub w_cursor: std::cell::Cell<CursorPos>,
    pub w_curswant: std::cell::Cell<i32>,
    pub w_set_curswant: std::cell::Cell<bool>,
    pub w_cursorline: std::cell::Cell<i32>,
    pub w_last_cursorline: std::cell::Cell<i32>,
    pub w_old_visual_mode: std::cell::Cell<u8>,
    pub w_old_cursor_lnum: std::cell::Cell<i32>,
    pub w_old_cursor_fcol: std::cell::Cell<i32>,
    pub w_old_cursor_lcol: std::cell::Cell<i32>,
    pub w_old_visual_lnum: std::cell::Cell<i32>,
    pub w_old_visual_col: std::cell::Cell<i32>,
    pub w_old_curswant: std::cell::Cell<i32>,
    pub w_last_cursor_lnum_rnu: std::cell::Cell<i32>,
    pub w_p_lcs_chars: std::cell::RefCell<LcsChars>,
    pub w_p_fcs_chars: std::cell::Cell<FcsChars>,
    pub w_topline: std::cell::Cell<i32>,
    pub w_topline_was_set: std::cell::Cell<bool>,
    pub w_topfill: std::cell::Cell<i32>,
    pub w_old_topfill: std::cell::Cell<i32>,
    pub w_botfill: std::cell::Cell<bool>,
    pub w_old_botfill: std::cell::Cell<bool>,
    pub w_leftcol: std::cell::Cell<i32>,
    pub w_skipcol: std::cell::Cell<i32>,
    pub w_last_topline: std::cell::Cell<i32>,
    pub w_last_topfill: std::cell::Cell<i32>,
    pub w_last_leftcol: std::cell::Cell<i32>,
    pub w_last_skipcol: std::cell::Cell<i32>,
    pub w_last_width: std::cell::Cell<i32>,
    pub w_last_height: std::cell::Cell<i32>,
    pub w_winrow: std::cell::Cell<i32>,
    pub w_height: std::cell::Cell<i32>,
    pub w_prev_winrow: std::cell::Cell<i32>,
    pub w_prev_height: std::cell::Cell<i32>,
    pub w_status_height: std::cell::Cell<i32>,
    pub w_winbar_height: std::cell::Cell<i32>,
    pub w_wincol: std::cell::Cell<i32>,
    pub w_width: std::cell::Cell<i32>,
    pub w_hsep_height: std::cell::Cell<i32>,
    pub w_vsep_width: std::cell::Cell<i32>,
    pub w_save_cursor: PosSave,
    pub w_do_win_fix_cursor: std::cell::Cell<bool>,
    pub w_wantline: std::cell::Cell<i32>,
    pub w_wantcol: std::cell::Cell<i32>,
    pub w_winrow_off: std::cell::Cell<i32>,
    pub w_wincol_off: std::cell::Cell<i32>,
    pub w_view_height: std::cell::Cell<i32>,
    pub w_view_width: std::cell::Cell<i32>,
    pub w_height_request: std::cell::Cell<i32>,
    pub w_width_request: std::cell::Cell<i32>,
    pub w_border_adj: [std::cell::Cell<i32>; 4],
    pub w_height_outer: std::cell::Cell<i32>,
    pub w_width_outer: std::cell::Cell<i32>,
    pub w_valid: std::cell::Cell<i32>,
    pub w_valid_cursor: std::cell::Cell<CursorPos>,
    pub w_valid_leftcol: std::cell::Cell<i32>,
    pub w_valid_skipcol: std::cell::Cell<i32>,
    pub w_topline_cursor_lnum: std::cell::Cell<Linenr>,
    pub w_viewport_invalid: std::cell::Cell<bool>,
    pub w_viewport_last_topline: std::cell::Cell<i32>,
    pub w_viewport_last_botline: std::cell::Cell<i32>,
    pub w_viewport_last_topfill: std::cell::Cell<i32>,
    pub w_viewport_last_skipcol: std::cell::Cell<i32>,
    pub w_cline_height: std::cell::Cell<i32>,
    pub w_cline_folded: std::cell::Cell<bool>,
    pub w_cline_row: std::cell::Cell<i32>,
    pub w_virtcol: std::cell::Cell<i32>,
    pub w_wrow: std::cell::Cell<i32>,
    pub w_wcol: std::cell::Cell<i32>,
    pub w_wcol_conceal_off: std::cell::Cell<i32>,
    pub w_botline: std::cell::Cell<i32>,
    pub w_empty_rows: std::cell::Cell<i32>,
    pub w_filler_rows: std::cell::Cell<i32>,
    pub w_lines_valid: std::cell::Cell<i32>,
    pub w_lines: std::cell::RefCell<Vec<crate::grid::Wline>>,
    pub w_folds: std::cell::RefCell<Vec<FoldEntry>>,
    pub w_fold_manual: std::cell::Cell<bool>,
    pub w_foldinvalid: std::cell::Cell<bool>,
    pub w_nrwidth: std::cell::Cell<i32>,
    pub w_scwidth: std::cell::Cell<i32>,
    pub w_minscwidth: std::cell::Cell<i32>,
    pub w_maxscwidth: std::cell::Cell<i32>,
    pub w_redr_type: std::cell::Cell<i32>,
    pub w_upd_rows: std::cell::Cell<i32>,
    pub w_redraw_top: std::cell::Cell<i32>,
    pub w_redraw_bot: std::cell::Cell<i32>,
    pub w_redr_status: std::cell::Cell<bool>,
    pub w_redr_border: std::cell::Cell<bool>,
    pub w_redr_statuscol: std::cell::Cell<bool>,
    pub w_display_tick: std::cell::Cell<u64>,
    pub w_stl_cursor: std::cell::Cell<CursorPos>,
    pub w_stl_virtcol: std::cell::Cell<i32>,
    pub w_stl_topline: std::cell::Cell<i32>,
    pub w_stl_line_count: std::cell::Cell<i32>,
    pub w_stl_topfill: std::cell::Cell<i32>,
    pub w_stl_empty: std::cell::Cell<bool>,
    pub w_stl_recording: std::cell::Cell<i32>,
    pub w_stl_state: std::cell::Cell<i32>,
    pub w_stl_visual_mode: std::cell::Cell<i32>,
    pub w_stl_visual_pos: std::cell::Cell<CursorPos>,
    pub w_alt_fnum: std::cell::Cell<i32>,
    pub w_alist: crate::globals::CloneCell<Option<std::rc::Rc<crate::arglist::Alist>>>,
    pub w_arg_idx: std::cell::Cell<i32>,
    pub w_arg_idx_invalid: std::cell::Cell<bool>,
    pub w_localdir: crate::globals::CloneCell<Option<std::ffi::CString>>,
    pub w_prevdir: crate::globals::CloneCell<Option<std::ffi::CString>>,
    pub w_onebuf_opt: Winopt,
    pub w_allbuf_opt: Winopt,
    pub w_p_cc_cols: std::cell::RefCell<Vec<i32>>,
    pub w_p_culopt_flags: std::cell::Cell<u8>,
    pub w_briopt_min: std::cell::Cell<i32>,
    pub w_briopt_shift: std::cell::Cell<i32>,
    pub w_briopt_sbr: std::cell::Cell<bool>,
    pub w_briopt_list: std::cell::Cell<i32>,
    pub w_briopt_vcol: std::cell::Cell<i32>,
    pub w_scbind_pos: std::cell::Cell<i32>,
    pub w_winvar: crate::eval::typval::DictitemId,
    pub w_pcmark: std::cell::Cell<CursorPos>,
    pub w_prev_pcmark: std::cell::Cell<CursorPos>,
    pub w_jumplist: std::cell::RefCell<[crate::mark::Xfmark; 100]>,
    pub w_jumplistlen: std::cell::Cell<i32>,
    pub w_jumplistidx: std::cell::Cell<i32>,
    pub w_changelistidx: std::cell::Cell<i32>,
    pub w_match_head: std::cell::RefCell<Vec<crate::r#match::Matchitem>>,
    pub w_next_match_id: std::cell::Cell<i32>,
    pub w_tagstack: std::cell::RefCell<[crate::tag::Taggy; 20]>,
    pub w_tagstackidx: std::cell::Cell<i32>,
    pub w_tagstacklen: std::cell::Cell<i32>,
    pub w_grid: GridView,
    pub w_grid_alloc: ScreenGrid,
    pub w_pos_changed: std::cell::Cell<bool>,
    pub w_floating: std::cell::Cell<bool>,
    pub w_kind: std::cell::Cell<i32>,
    pub w_config: std::cell::RefCell<WinConfig>,
    pub w_fraction: std::cell::Cell<i32>,
    pub w_prev_fraction_row: std::cell::Cell<i32>,
    pub w_nrwidth_line_count: std::cell::Cell<i32>,
    pub w_statuscol_line_count: std::cell::Cell<i32>,
    pub w_nrwidth_width: std::cell::Cell<i32>,
    pub w_llist: crate::globals::CloneCell<crate::quickfix::QfInfoId>,
    pub w_llist_ref: crate::globals::CloneCell<crate::quickfix::QfInfoId>,
    pub w_status_click_defs: std::cell::RefCell<Vec<crate::mouse::StlClickDefinition>>,
    pub w_winbar_click_defs: std::cell::RefCell<Vec<crate::mouse::StlClickDefinition>>,
    pub w_statuscol_click_defs: std::cell::RefCell<std::collections::BTreeMap<i32, StcClicks>>,
}

impl WinInfoInfo {
    #[inline]
    pub fn frame_mut(&self) -> crate::globals::FramePtr {
        self.w_frame.get().clone()
    }

    #[inline]
    pub fn next_opt(&self) -> Option<crate::globals::WinPtr> {
        let n = self.w_next.get();
        (!n.is_null()).then_some(n)
    }

    #[inline]
    pub fn prev_mut(&self) -> crate::globals::WinPtr {
        self.w_prev.get()
    }

    #[inline]
    pub fn prev_opt(&self) -> Option<crate::globals::WinPtr> {
        let p = self.w_prev.get();
        (!p.is_null()).then_some(p)
    }

    #[inline]
    pub fn buf_mut(&self) -> crate::globals::BufPtr {
        self.w_buffer.get()
    }

    #[inline(always)]
    pub fn with_buf<R>(&self, f: impl FnOnce(&crate::buffer::FileBuf) -> R) -> R {
        self.w_buffer.with(|b| f(b))
    }

    #[inline]
    pub fn buf_opt(&self) -> Option<crate::globals::BufPtr> {
        let b = self.w_buffer.get();
        (!b.is_null()).then_some(b)
    }

    pub const fn new() -> Self {
        const ZERO_POS: CursorPos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        Self {
            self_weak: std::rc::Weak::new(),
            handle: 0,
            w_buffer: crate::globals::CloneCell::new(crate::globals::BufPtr::NULL),
            w_s_owned: crate::globals::CloneCell::new(None),
            w_ns_hl: std::cell::Cell::new(0),
            w_ns_hl_winhl: std::cell::Cell::new(0),
            w_ns_hl_active: std::cell::Cell::new(0),
            w_ns_hl_attr: crate::globals::CloneCell::new(None),
            w_ns_set: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            w_hl_id_normal: std::cell::Cell::new(0),
            w_hl_attr_normal: std::cell::Cell::new(0),
            w_hl_attr_normalnc: std::cell::Cell::new(0),
            w_hl_needs_update: std::cell::Cell::new(0),
            w_prev: crate::globals::CloneCell::new(crate::globals::WinPtr::NULL),
            w_next: crate::globals::CloneCell::new(crate::globals::WinPtr::NULL),
            w_locked: std::cell::Cell::new(0),
            w_frame: crate::globals::CloneCell::new(crate::globals::FramePtr::NULL),
            w_cursor: std::cell::Cell::new(CursorPos {
                lnum: 1,
                col: 0,
                coladd: 0,
            }),
            w_curswant: std::cell::Cell::new(0),
            w_set_curswant: std::cell::Cell::new(false),
            w_cursorline: std::cell::Cell::new(0),
            w_last_cursorline: std::cell::Cell::new(0),
            w_old_visual_mode: std::cell::Cell::new(0),
            w_old_cursor_lnum: std::cell::Cell::new(0),
            w_old_cursor_fcol: std::cell::Cell::new(0),
            w_old_cursor_lcol: std::cell::Cell::new(0),
            w_old_visual_lnum: std::cell::Cell::new(0),
            w_old_visual_col: std::cell::Cell::new(0),
            w_old_curswant: std::cell::Cell::new(0),
            w_last_cursor_lnum_rnu: std::cell::Cell::new(0),
            w_p_lcs_chars: std::cell::RefCell::new(LcsChars::new()),
            w_p_fcs_chars: std::cell::Cell::new(FcsChars {
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
            }),
            w_topline: std::cell::Cell::new(1),
            w_topline_was_set: std::cell::Cell::new(false),
            w_topfill: std::cell::Cell::new(0),
            w_old_topfill: std::cell::Cell::new(0),
            w_botfill: std::cell::Cell::new(false),
            w_old_botfill: std::cell::Cell::new(false),
            w_leftcol: std::cell::Cell::new(0),
            w_skipcol: std::cell::Cell::new(0),
            w_last_topline: std::cell::Cell::new(0),
            w_last_topfill: std::cell::Cell::new(0),
            w_last_leftcol: std::cell::Cell::new(0),
            w_last_skipcol: std::cell::Cell::new(0),
            w_last_width: std::cell::Cell::new(0),
            w_last_height: std::cell::Cell::new(0),
            w_winrow: std::cell::Cell::new(0),
            w_height: std::cell::Cell::new(0),
            w_prev_winrow: std::cell::Cell::new(0),
            w_prev_height: std::cell::Cell::new(0),
            w_status_height: std::cell::Cell::new(0),
            w_winbar_height: std::cell::Cell::new(0),
            w_wincol: std::cell::Cell::new(0),
            w_width: std::cell::Cell::new(0),
            w_hsep_height: std::cell::Cell::new(0),
            w_vsep_width: std::cell::Cell::new(0),
            w_save_cursor: PosSave {
                w_topline_save: std::cell::Cell::new(0),
                w_topline_corr: std::cell::Cell::new(0),
                w_cursor_save: std::cell::Cell::new(ZERO_POS),
                w_cursor_corr: std::cell::Cell::new(ZERO_POS),
            },
            w_do_win_fix_cursor: std::cell::Cell::new(false),
            w_wantline: std::cell::Cell::new(0),
            w_wantcol: std::cell::Cell::new(0),
            w_winrow_off: std::cell::Cell::new(0),
            w_wincol_off: std::cell::Cell::new(0),
            w_view_height: std::cell::Cell::new(0),
            w_view_width: std::cell::Cell::new(0),
            w_height_request: std::cell::Cell::new(0),
            w_width_request: std::cell::Cell::new(0),
            w_border_adj: [const { std::cell::Cell::new(0) }; 4],
            w_height_outer: std::cell::Cell::new(0),
            w_width_outer: std::cell::Cell::new(0),
            w_valid: std::cell::Cell::new(0),
            w_valid_cursor: std::cell::Cell::new(ZERO_POS),
            w_valid_leftcol: std::cell::Cell::new(0),
            w_valid_skipcol: std::cell::Cell::new(0),
            w_topline_cursor_lnum: std::cell::Cell::new(0),
            w_viewport_invalid: std::cell::Cell::new(true),
            w_viewport_last_topline: std::cell::Cell::new(1),
            w_viewport_last_botline: std::cell::Cell::new(0),
            w_viewport_last_topfill: std::cell::Cell::new(0),
            w_viewport_last_skipcol: std::cell::Cell::new(0),
            w_cline_height: std::cell::Cell::new(0),
            w_cline_folded: std::cell::Cell::new(false),
            w_cline_row: std::cell::Cell::new(0),
            w_virtcol: std::cell::Cell::new(0),
            w_wrow: std::cell::Cell::new(0),
            w_wcol: std::cell::Cell::new(0),
            w_wcol_conceal_off: std::cell::Cell::new(0),
            w_botline: std::cell::Cell::new(2),
            w_empty_rows: std::cell::Cell::new(0),
            w_filler_rows: std::cell::Cell::new(0),
            w_lines_valid: std::cell::Cell::new(0),
            w_lines: std::cell::RefCell::new(Vec::new()),
            w_folds: std::cell::RefCell::new(Vec::new()),
            w_fold_manual: std::cell::Cell::new(false),
            w_foldinvalid: std::cell::Cell::new(false),
            w_nrwidth: std::cell::Cell::new(0),
            w_scwidth: std::cell::Cell::new(0),
            w_minscwidth: std::cell::Cell::new(0),
            w_maxscwidth: std::cell::Cell::new(0),
            w_redr_type: std::cell::Cell::new(0),
            w_upd_rows: std::cell::Cell::new(0),
            w_redraw_top: std::cell::Cell::new(0),
            w_redraw_bot: std::cell::Cell::new(0),
            w_redr_status: std::cell::Cell::new(false),
            w_redr_border: std::cell::Cell::new(false),
            w_redr_statuscol: std::cell::Cell::new(false),
            w_display_tick: std::cell::Cell::new(0),
            w_stl_cursor: std::cell::Cell::new(ZERO_POS),
            w_stl_virtcol: std::cell::Cell::new(0),
            w_stl_topline: std::cell::Cell::new(0),
            w_stl_line_count: std::cell::Cell::new(0),
            w_stl_topfill: std::cell::Cell::new(0),
            w_stl_empty: std::cell::Cell::new(false),
            w_stl_recording: std::cell::Cell::new(0),
            w_stl_state: std::cell::Cell::new(0),
            w_stl_visual_mode: std::cell::Cell::new(0),
            w_stl_visual_pos: std::cell::Cell::new(ZERO_POS),
            w_alt_fnum: std::cell::Cell::new(0),
            w_alist: crate::globals::CloneCell::new(None),
            w_arg_idx: std::cell::Cell::new(0),
            w_arg_idx_invalid: std::cell::Cell::new(false),
            w_localdir: crate::globals::CloneCell::new(None),
            w_prevdir: crate::globals::CloneCell::new(None),
            w_onebuf_opt: Winopt::ZERO,
            w_allbuf_opt: Winopt::ZERO,
            w_p_cc_cols: std::cell::RefCell::new(Vec::new()),
            w_p_culopt_flags: std::cell::Cell::new(0),
            w_briopt_min: std::cell::Cell::new(0),
            w_briopt_shift: std::cell::Cell::new(0),
            w_briopt_sbr: std::cell::Cell::new(false),
            w_briopt_list: std::cell::Cell::new(0),
            w_briopt_vcol: std::cell::Cell::new(0),
            w_scbind_pos: std::cell::Cell::new(1),
            w_winvar: crate::eval::typval::DictitemId::NULL,
            w_pcmark: std::cell::Cell::new(ZERO_POS),
            w_prev_pcmark: std::cell::Cell::new(ZERO_POS),
            w_jumplist: std::cell::RefCell::new([const { crate::mark::Xfmark::ZERO }; 100]),
            w_jumplistlen: std::cell::Cell::new(0),
            w_jumplistidx: std::cell::Cell::new(0),
            w_changelistidx: std::cell::Cell::new(0),
            w_match_head: std::cell::RefCell::new(Vec::new()),
            w_next_match_id: std::cell::Cell::new(0),
            w_tagstack: std::cell::RefCell::new([const { crate::tag::Taggy::ZERO }; 20]),
            w_tagstackidx: std::cell::Cell::new(0),
            w_tagstacklen: std::cell::Cell::new(0),
            w_grid: GridView {
                target: std::cell::Cell::new(GridTarget::None),
                row_offset: std::cell::Cell::new(0),
                col_offset: std::cell::Cell::new(0),
            },
            w_grid_alloc: SCREEN_GRID_INIT,
            w_pos_changed: std::cell::Cell::new(false),
            w_floating: std::cell::Cell::new(false),
            w_kind: std::cell::Cell::new(K_WIN_NORMAL),
            w_config: std::cell::RefCell::new(WIN_CONFIG_INIT),
            w_fraction: std::cell::Cell::new(0),
            w_prev_fraction_row: std::cell::Cell::new(0),
            w_nrwidth_line_count: std::cell::Cell::new(0),
            w_statuscol_line_count: std::cell::Cell::new(0),
            w_nrwidth_width: std::cell::Cell::new(0),
            w_llist: crate::globals::CloneCell::new(crate::quickfix::QfInfoId::NULL),
            w_llist_ref: crate::globals::CloneCell::new(crate::quickfix::QfInfoId::NULL),
            w_status_click_defs: std::cell::RefCell::new(Vec::new()),
            w_winbar_click_defs: std::cell::RefCell::new(Vec::new()),
            w_statuscol_click_defs: std::cell::RefCell::new(std::collections::BTreeMap::new()),
        }
    }

    #[inline]
    pub fn w_s_ptr(&self) -> std::rc::Rc<crate::syntax::Synblock> {
        self.w_s_owned
            .get()
            .unwrap_or_else(|| self.buf_mut().b_s.clone())
    }

    #[inline]
    pub fn w_vars(&self) -> crate::eval::typval::DictId {
        self.w_winvar.tv().vval.v_dict()
    }
}

pub type StcClick = Vec<crate::mouse::StlClickDefinition>;
pub type StcClicks = std::collections::BTreeMap<i32, StcClick>;

pub struct SnapshotFrame {
    pub fr_layout: u8,
    pub fr_width: i32,
    pub fr_height: i32,
    pub fr_next: Option<Box<SnapshotFrame>>,
    pub fr_child: Option<Box<SnapshotFrame>>,
    pub fr_win: crate::globals::WinPtr,
}

#[repr(C)]
pub struct TabPageInfo {
    pub self_weak: std::rc::Weak<Self>,
    pub handle: ObjHandle,
    pub tp_next: crate::api::private::validate::CloneCell<crate::globals::TabpagePtr>,
    pub tp_topframe: crate::api::private::validate::CloneCell<crate::globals::FramePtr>,
    pub tp_curwin: crate::globals::CloneCell<crate::globals::WinPtr>,
    pub tp_prevwin: crate::globals::CloneCell<crate::globals::WinPtr>,
    pub tp_firstwin: crate::globals::CloneCell<crate::globals::WinPtr>,
    pub tp_lastwin: crate::globals::CloneCell<crate::globals::WinPtr>,
    pub tp_old_rows_avail: std::cell::Cell<i64>,
    pub tp_old_columns: std::cell::Cell<i64>,
    pub tp_ch_used: std::cell::Cell<i64>,
    pub tp_did_tabclosedpre: std::cell::Cell<bool>,
    pub tp_diffs: std::cell::RefCell<Vec<crate::diff::DiffBlock>>,
    pub tp_diffbuf: [crate::globals::CloneCell<crate::globals::BufPtr>; 8],
    pub tp_diff_invalid: std::cell::Cell<i32>,
    pub tp_diff_update: std::cell::Cell<i32>,
    pub tp_snapshot: [std::cell::RefCell<Option<Box<SnapshotFrame>>>; 3],
    pub tp_winvar: crate::api::private::validate::CloneCell<crate::eval::typval::DictitemId>,
    pub tp_localdir: crate::api::private::validate::CloneCell<Option<std::ffi::CString>>,
    pub tp_prevdir: crate::api::private::validate::CloneCell<Option<std::ffi::CString>>,
}

impl TabPageInfo {
    #[inline]
    pub fn diffbuf_mut(&self, idx: usize) -> crate::globals::BufPtr {
        self.tp_diffbuf[idx].get()
    }

    #[inline]
    pub fn diffbuf_opt(&self, idx: usize) -> Option<crate::globals::BufPtr> {
        let b = self.tp_diffbuf[idx].get();
        (!b.is_null()).then_some(b)
    }

    #[inline]
    pub fn next_opt(&self) -> Option<crate::globals::TabpagePtr> {
        let n = self.tp_next.get();
        (!n.is_null()).then_some(n)
    }

    #[inline]
    pub fn topframe_mut(&self) -> crate::globals::FramePtr {
        self.tp_topframe.get()
    }

    #[inline]
    pub fn firstwin_mut(&self) -> crate::globals::WinPtr {
        self.tp_firstwin.get()
    }

    #[inline]
    pub fn lastwin_mut(&self) -> crate::globals::WinPtr {
        self.tp_lastwin.get()
    }

    #[inline]
    pub fn curwin_mut(&self) -> crate::globals::WinPtr {
        self.tp_curwin.get()
    }

    pub fn new_rc(handle: ObjHandle) -> std::rc::Rc<Self> {
        std::rc::Rc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            handle,
            tp_next: crate::api::private::validate::CloneCell::new(
                crate::globals::TabpagePtr::NULL,
            ),
            tp_topframe: crate::api::private::validate::CloneCell::new(
                crate::globals::FramePtr::NULL,
            ),
            tp_curwin: crate::globals::CloneCell::new(crate::globals::WinPtr::NULL),
            tp_prevwin: crate::globals::CloneCell::new(crate::globals::WinPtr::NULL),
            tp_firstwin: crate::globals::CloneCell::new(crate::globals::WinPtr::NULL),
            tp_lastwin: crate::globals::CloneCell::new(crate::globals::WinPtr::NULL),
            tp_old_rows_avail: std::cell::Cell::new(0),
            tp_old_columns: std::cell::Cell::new(0),
            tp_ch_used: std::cell::Cell::new(0),
            tp_did_tabclosedpre: std::cell::Cell::new(false),
            tp_diffs: std::cell::RefCell::new(Vec::new()),
            tp_diffbuf: [const { crate::globals::CloneCell::new(crate::globals::BufPtr::NULL) }; 8],
            tp_diff_invalid: std::cell::Cell::new(1),
            tp_diff_update: std::cell::Cell::new(0),
            tp_snapshot: [
                std::cell::RefCell::new(None),
                std::cell::RefCell::new(None),
                std::cell::RefCell::new(None),
            ],
            tp_winvar: crate::api::private::validate::CloneCell::new(
                crate::eval::typval::DictitemId::NULL,
            ),
            tp_localdir: crate::api::private::validate::CloneCell::new(None),
            tp_prevdir: crate::api::private::validate::CloneCell::new(None),
        })
    }

    #[inline]
    pub fn tp_vars(&self) -> crate::eval::typval::DictId {
        self.tp_winvar.get().tv().vval.v_dict()
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

pub const WSP_ROOM: i32 = 0x01;
pub const WSP_VERT: i32 = 0x02;
pub const WSP_HOR: i32 = 0x04;
pub const WSP_TOP: i32 = 0x08;
pub const WSP_BOT: i32 = 0x10;
pub const WSP_HELP: i32 = 0x20;
pub const WSP_BELOW: i32 = 0x40;
pub const WSP_ABOVE: i32 = 0x80;
pub const WSP_NEWLOC: i32 = 0x100;
pub const WSP_NOENTER: i32 = 0x200;
pub const WSP_QUICKFIX: i32 = 0x400;

pub const MIN_COLUMNS: i32 = 12;
pub const MIN_LINES: i32 = 2;
pub const STATUS_HEIGHT: i32 = 1;
pub const LOWEST_WIN_ID: i32 = 1000;

const SNAP_HELP_IDX: usize = 0;
const SNAP_QUICKFIX_IDX: usize = 2;
const SNAP_COUNT: usize = 3;

const FRACTION_MULT: i32 = 16384;

const WEE_UNDO_SYNC: i32 = 0x01;
const WEE_CURWIN_INVALID: i32 = 0x02;
const WEE_TRIGGER_NEW_AUTOCMDS: i32 = 0x04;
const WEE_TRIGGER_ENTER_AUTOCMDS: i32 = 0x08;
const WEE_TRIGGER_LEAVE_AUTOCMDS: i32 = 0x10;

const K_OPT_TCL_FLAG_LEFT: u32 = 0x01;
const K_OPT_TCL_FLAG_USELAST: u32 = 0x02;

const K_WIN_OPT_SCROLL: usize = 33;

const K_WIN_NORMAL: WinKind = 0;

const E_CANNOT_CLOSE_LAST_WINDOW: &CStr = c"E444: Cannot close last window";
const E_CANNOT_SPLIT_WINDOW_WHEN_CLOSING_BUFFER: &CStr =
    c"E1159: Cannot split a window when closing the buffer";
const M_ONLYONE: &CStr = c"Already only one window";

struct WindowModuleState {
    split_disallowed: std::cell::Cell<i32>,
    close_disallowed: std::cell::Cell<i32>,
    frame_locked: std::cell::Cell<i32>,
    min_set_ch: std::cell::Cell<OptInt>,
    command_frame_height: std::cell::Cell<bool>,
    last_tp_handle: std::cell::Cell<i32>,
    last_win_id: std::cell::Cell<i32>,
    old_rows: std::cell::Cell<i32>,
    old_columns: std::cell::Cell<i32>,
    did_initial_scroll_size_snapshot: std::cell::Cell<bool>,
    winclosed_recursive: std::cell::Cell<bool>,
    tabclosedpre_recursive: std::cell::Cell<bool>,
    win_scrolled_resized_recursive: std::cell::Cell<bool>,
}

impl WindowModuleState {
    const fn new() -> Self {
        Self {
            split_disallowed: std::cell::Cell::new(0),
            close_disallowed: std::cell::Cell::new(0),
            frame_locked: std::cell::Cell::new(0),
            min_set_ch: std::cell::Cell::new(1),
            command_frame_height: std::cell::Cell::new(true),
            last_tp_handle: std::cell::Cell::new(0),
            last_win_id: std::cell::Cell::new(LOWEST_WIN_ID - 1),
            old_rows: std::cell::Cell::new(0),
            old_columns: std::cell::Cell::new(0),
            did_initial_scroll_size_snapshot: std::cell::Cell::new(false),
            winclosed_recursive: std::cell::Cell::new(false),
            tabclosedpre_recursive: std::cell::Cell::new(false),
            win_scrolled_resized_recursive: std::cell::Cell::new(false),
        }
    }
}

thread_local! {
    static WIN_STATE: WindowModuleState = const { WindowModuleState::new() };
}

const ERROR_INIT: Error = Error::new();

#[inline]
fn nowin() -> Option<crate::globals::WinPtr> {
    None
}

#[inline]
fn rows_avail() -> OptInt {
    OptInt::from(g_st().rows_global.get())
        - opt_vars().p_ch.get()
        - OptInt::from(tabline_height())
        - OptInt::from(global_stl_height())
}

#[inline]
fn one_window_macro() -> bool {
    ptr::eq(
        g_st().firstwin.get().as_ptr(),
        g_st().lastwin.get().as_ptr(),
    )
}

#[inline]
pub fn reset_binding(wp: &WinInfoInfo) {
    wp.w_onebuf_opt.wo_scb.set(false);
    wp.w_onebuf_opt.wo_crb.set(false);
}

/// Disallow changing the window layout (split window, close window, move
/// window). Resizing is still allowed.
pub fn window_layout_lock() {
    WIN_STATE.with(|s| s.split_disallowed.set(s.split_disallowed.get() + (1)));
    WIN_STATE.with(|s| s.close_disallowed.set(s.close_disallowed.get() + (1)));
}

pub fn window_layout_unlock() {
    WIN_STATE.with(|s| s.split_disallowed.set(s.split_disallowed.get() - (1)));
    WIN_STATE.with(|s| s.close_disallowed.set(s.close_disallowed.get() - (1)));
}

pub fn frames_locked() -> bool {
    WIN_STATE.with(|s| s.frame_locked.get()) != 0
}

/// When the window layout cannot be changed give an error and return true.
pub fn window_layout_locked(cmd: Cmdidx) -> bool {
    let mut err = ERROR_INIT;
    let locked = window_layout_locked_err(cmd, &mut err);
    if error_set(&err) {
        emsg(gettext((err.msg_cstr()).unwrap_or(c"")));
        api_clear_error(&mut err);
    }
    locked
}

/// Like `window_layout_locked`, but set `err` to the (untranslated) error message when locked.
pub fn window_layout_locked_err(cmd: Cmdidx, err: &mut Error) -> bool {
    if WIN_STATE.with(|s| s.split_disallowed.get()) > 0
        || WIN_STATE.with(|s| s.close_disallowed.get()) > 0
    {
        if WIN_STATE.with(|s| s.close_disallowed.get()) == 0 && cmd == Cmdidx::CmdTabnew {
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

/// Check if the current window is allowed to move to a different buffer.
pub fn check_can_set_curbuf_disabled() -> bool {
    if curwin_mut().w_onebuf_opt.wo_wfb.get() {
        emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
        return false;
    }
    true
}

/// Check if the current window is allowed to move to a different buffer.
pub fn check_can_set_curbuf_forceit(forceit: bool) -> bool {
    if !forceit && curwin_mut().w_onebuf_opt.wo_wfb.get() {
        emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
        return false;
    }
    true
}

/// Return the current window, unless in the cmdline window and "prevwin" is
/// set, then return "prevwin".
pub fn prevwin_curwin() -> crate::globals::WinPtr {
    if is_in_cmdwin()
        && let Some(pw) = prevwin_opt()
    {
        pw
    } else {
        curwin_mut()
    }
}

/// If the 'switchbuf' option contains "useopen" or "usetab", then try to jump
/// to a window containing "buf".
/// Returns the pointer to the window that was jumped to or NULL.
pub fn swbuf_goto_win_with_buf(buf: Option<&FileBuf>) -> Option<crate::globals::WinPtr> {
    let buf_ref = buf?;
    let buf_ptr = crate::globals::BufPtr::from(buf_ref);
    let mut wp: Option<crate::globals::WinPtr> = None;

    if (opt_vars().swb_flags.get() & K_OPT_SWB_FLAG_USEOPEN) != 0 {
        wp = buf_jump_open_win(buf_ptr.borrow_mut());
    }

    if wp.is_none() && (opt_vars().swb_flags.get() & K_OPT_SWB_FLAG_USETAB) != 0 {
        wp = buf_jump_open_tab(buf_ptr.borrow_mut());
    }

    wp
}

#[allow(non_snake_case)]
fn do_window_newwindow(nchar: i32, Prenum: i32, cbuf: &mut [u8; 40]) {
    if Prenum != 0 {
        let max_len = cbuf.len() - 5;
        format_to_buf!(&mut cbuf[..max_len], c"%lld", i64::from(Prenum));
    } else {
        cbuf[0] = 0;
    }
    if nchar == b'v' as i32 || nchar == CTRL_V_CONST {
        crate::memory::append_bytes_to_c_buf(cbuf, b"v");
    }
    crate::memory::append_bytes_to_c_buf(cbuf, b"new");
    do_cmdline_cmd(crate::memory::bytes_as_cstr(cbuf));
}

fn do_window_gotofile(nchar: i32, prenum1: i32) {
    if check_text_or_curbuf_locked(None) {
        return;
    }

    let mut lnum: Linenr = -1;
    if let Some(fname_cstr) = grab_file_name(prenum1, &mut lnum) {
        let oldtab = g_st().curtab.get().clone();
        let oldwin = curwin_mut();
        setpcmark();

        let mut wp = crate::globals::WinPtr::NULL;
        if (opt_vars().swb_flags.get() & (K_OPT_SWB_FLAG_USEOPEN | K_OPT_SWB_FLAG_USETAB)) != 0
            && g_st().cmdmod.borrow().cmod_tab == 0
        {
            let found = buflist_findname_exp(Some(&fname_cstr));
            wp = crate::globals::WinPtr::from(swbuf_goto_win_with_buf(found.as_deref()));
        }

        if wp.is_null() && win_split(0, 0) {
            reset_binding(&curwin_mut());
            if !do_ecmd(
                0,
                Some(&fname_cstr),
                None,
                None,
                ECMD_LASTL,
                ECMD_HIDE,
                None,
            ) {
                let old_got_int = g_st().got_int.get();
                g_st().got_int.set(false);

                win_close(&curwin_mut(), false, false);
                g_st().got_int.set(g_st().got_int.get() || old_got_int);

                goto_tabpage_win(oldtab.borrow(), &oldwin);
            } else {
                wp = curwin_ptr();
            }
        }

        if !wp.is_null() && nchar == b'F' as i32 && lnum >= 0 {
            curwin_mut().w_cursor.set_lnum(lnum);
            check_cursor_lnum(&curwin_mut());
            beginline(BL_SOL | BL_FIX);
        }
    }
}

/// All CTRL-W window commands are handled here, called from normal_cmd().
pub fn do_window(mut nchar: i32, mut prenum: i32, mut xchar: i32) {
    let mut type_ = FIND_DEFINE;
    let mut cbuf = [0u8; 40];
    let prenum1 = if prenum == 0 { 1 } else { prenum };

    match nchar {
        c if c == b'S' as i32 || c == CTRL_S_CONST || c == b's' as i32 => {
            reset_visual_and_resel();
            if bt_quickfix(Some(&crate::globals::curbuf_mut())) {
                do_window_newwindow(nchar, prenum, &mut cbuf);
            } else {
                win_split(prenum, 0);
            }
        }
        c if c == CTRL_V_CONST || c == b'v' as i32 => {
            reset_visual_and_resel();
            if bt_quickfix(Some(&crate::globals::curbuf_mut())) {
                do_window_newwindow(nchar, prenum, &mut cbuf);
            } else {
                win_split(prenum, WSP_VERT);
            }
        }
        c if c == CTRL_HAT_CONST || c == b'^' as i32 => {
            reset_visual_and_resel();

            if buflist_findnr(if prenum == 0 {
                curwin_mut().w_alt_fnum.get()
            } else {
                prenum
            })
            .is_none()
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
                        curwin_mut().w_alt_fnum.get()
                    } else {
                        prenum
                    },
                    0,
                    GETF_ALT,
                    0,
                );
            }
        }
        c if c == CTRL_N || c == b'n' as i32 => {
            reset_visual_and_resel();
            do_window_newwindow(nchar, prenum, &mut cbuf);
        }
        c if c == CTRL_Q_CONST || c == b'q' as i32 => {
            reset_visual_and_resel();
            cmd_with_count(c"quit", &mut cbuf, i64::from(prenum));
            do_cmdline_cmd(crate::memory::bytes_as_cstr(&cbuf));
        }
        c if c == CTRL_C_CONST || c == b'c' as i32 => {
            reset_visual_and_resel();
            cmd_with_count(c"close", &mut cbuf, i64::from(prenum));
            do_cmdline_cmd(crate::memory::bytes_as_cstr(&cbuf));
        }
        c if c == CTRL_Z_CONST || c == b'z' as i32 => {
            reset_visual_and_resel();
            do_cmdline_cmd(c"pclose");
        }
        c if c == b'P' as i32 => {
            let mut wp = crate::globals::WinPtr::NULL;
            for wp2 in for_all_windows_ptr() {
                if wp2.w_onebuf_opt.wo_pvw.get() {
                    wp = wp2;
                    break;
                }
            }
            if wp.is_null() {
                emsg(gettext(c"E441: There is no preview window"));
            } else {
                win_goto(wp.borrow_mut());
            }
        }
        c if c == CTRL_O || c == b'o' as i32 => {
            reset_visual_and_resel();
            cmd_with_count(c"only", &mut cbuf, i64::from(prenum));
            do_cmdline_cmd(crate::memory::bytes_as_cstr(&cbuf));
        }
        c if c == CTRL_W_CONST || c == b'w' as i32 || c == b'W' as i32 => {
            if one_window_macro() && prenum != 1 {
                beep_flush();
            } else {
                let wp = if prenum != 0 {
                    let mut last_focusable = g_st().firstwin.get();
                    let mut cur = firstwin_mut();
                    while {
                        prenum -= 1;
                        prenum > 0
                    } {
                        if !cur.w_floating.get() || win_is_focusable(&cur) {
                            last_focusable = cur.clone();
                        }
                        let Some(next) = cur.next_opt() else {
                            break;
                        };
                        cur = next;
                    }
                    let mut opt = Some(cur);
                    while let Some(ref w) = opt
                        && w.w_floating.get()
                        && !win_is_focusable(w)
                    {
                        opt = w.next_opt();
                    }
                    opt.map_or(last_focusable, crate::globals::WinPtr::from)
                } else if nchar == b'W' as i32 {
                    let mut opt = curwin_mut().prev_opt().or_else(|| Some(lastwin_mut()));
                    while let Some(ref w) = opt
                        && w.w_floating.get()
                        && !win_is_focusable(w)
                    {
                        opt = w.prev_opt();
                    }
                    opt.unwrap_or(crate::globals::WinPtr::NULL)
                } else {
                    let mut opt = curwin_mut().next_opt();
                    while let Some(ref w) = opt
                        && w.w_floating.get()
                        && !win_is_focusable(w)
                    {
                        opt = w.next_opt();
                    }
                    opt.map_or(g_st().firstwin.get(), crate::globals::WinPtr::from)
                };
                win_goto(wp.borrow_mut());
            }
        }
        c if c == b'j' as i32 || c == K_DOWN || c == CTRL_J_CONST => {
            win_goto_ver(false, prenum1);
        }
        c if c == b'k' as i32 || c == K_UP || c == CTRL_K_CONST => {
            win_goto_ver(true, prenum1);
        }
        c if c == b'h' as i32 || c == K_LEFT || c == CTRL_H_CONST || c == K_BS => {
            win_goto_hor(true, prenum1);
        }
        c if c == b'l' as i32 || c == K_RIGHT || c == CTRL_L_CONST => {
            win_goto_hor(false, prenum1);
        }
        c if c == b'T' as i32 => {
            if one_window(&curwin_mut(), None) {
                msg(gettext(M_ONLYONE), 0);
            } else {
                let oldtab = g_st().curtab.get().clone();
                let wp = curwin_ptr();
                if win_new_tabpage(prenum, None, true, None).is_some()
                    && valid_tabpage(oldtab.opt())
                {
                    let newtab = g_st().curtab.get().clone();
                    goto_tabpage_tp(oldtab.opt(), true, true);
                    if wp == curwin_ptr() {
                        win_close(&curwin_mut(), false, false);
                    }
                    if valid_tabpage(newtab.opt()) {
                        goto_tabpage_tp(newtab.opt(), true, true);
                        apply_autocmds(
                            EVENT_TABNEWENTERED,
                            None,
                            None,
                            false,
                            curbuf_opt_mut().as_deref(),
                        );
                    }
                }
            }
        }
        c if c == b't' as i32 || c == CTRL_T_CONST => {
            win_goto(&firstwin_mut());
        }
        c if c == b'b' as i32 || c == CTRL_B_CONST => {
            win_goto(&lastwin_nofloating(None));
        }
        c if c == b'p' as i32 || c == CTRL_P => {
            if !win_valid(g_st().prevwin.get().opt())
                || !win_is_focusable(&crate::globals::prevwin_mut())
            {
                beep_flush();
            } else {
                win_goto(&crate::globals::prevwin_mut());
            }
        }
        c if c == b'x' as i32 || c == CTRL_X_CONST => {
            win_exchange(prenum);
        }
        c if c == CTRL_R_CONST || c == b'r' as i32 => {
            reset_visual_and_resel();
            win_rotate(false, prenum1);
        }
        c if c == b'R' as i32 => {
            reset_visual_and_resel();
            win_rotate(true, prenum1);
        }
        c if c == b'K' as i32 || c == b'J' as i32 || c == b'H' as i32 || c == b'L' as i32 => {
            if one_window(&curwin_mut(), None) {
                beep_flush();
            } else {
                let dir = (if nchar == b'H' as i32 || nchar == b'L' as i32 {
                    WSP_VERT
                } else {
                    0
                }) | (if nchar == b'H' as i32 || nchar == b'K' as i32 {
                    WSP_TOP
                } else {
                    WSP_BOT
                });
                win_splitmove(&curwin_mut(), prenum, dir);
            }
        }
        c if c == b'=' as i32 => {
            let mod_ = g_st().cmdmod.borrow().cmod_split & (WSP_VERT | WSP_HOR);
            win_equal(
                None,
                false,
                if mod_ == WSP_VERT {
                    b'v' as i32
                } else if mod_ == WSP_HOR {
                    b'h' as i32
                } else {
                    b'b' as i32
                },
            );
        }
        c if c == b'+' as i32 => {
            win_setheight(curwin_mut().w_height.get() + prenum1);
        }
        c if c == b'-' as i32 => {
            win_setheight(curwin_mut().w_height.get() - prenum1);
        }
        c if c == CTRL_CONST_CONST || c == b'_' as i32 => {
            win_setheight(if prenum != 0 {
                prenum
            } else {
                g_st().rows_global.get() - WIN_STATE.with(|s| s.min_set_ch.get()) as i32
            });
        }
        c if c == b'>' as i32 => {
            win_setwidth(curwin_mut().w_width.get() + prenum1);
        }
        c if c == b'<' as i32 => {
            win_setwidth(curwin_mut().w_width.get() - prenum1);
        }
        c if c == b'|' as i32 => {
            win_setwidth(if prenum != 0 {
                prenum
            } else {
                g_st().columns_global.get()
            });
        }
        c if c == b'}' as i32 || c == b']' as i32 || c == CTRL_RSB_CONST => {
            if nchar == b'}' as i32 {
                if prenum != 0 {
                    g_st().g_do_tagpreview.set(prenum);
                } else {
                    g_st().g_do_tagpreview.set(opt_vars().p_pvh.get() as i32);
                }
            } else {
                g_st().g_do_tagpreview.set(0);
            }

            if prenum != 0 {
                g_st().postponed_split.set(prenum);
            } else {
                g_st().postponed_split.set(-1);
            }

            do_nv_ident(CTRL_RSB_CONST, NUL as i32);
            g_st().postponed_split.set(0);
        }
        c if c == b'f' as i32 || c == b'F' as i32 || c == CTRL_F_CONST => {
            do_window_gotofile(nchar, prenum1);
        }
        c if c == b'i' as i32 || c == CTRL_I_CONST || c == b'd' as i32 || c == CTRL_D_CONST => {
            if nchar == b'i' as i32 || nchar == CTRL_I_CONST {
                type_ = FIND_ANY;
            }
            let ident = find_ident_under_cursor(FIND_IDENT, None);
            if ident.is_empty() {
                return;
            }
            find_pattern_in_path(
                &ident,
                0,
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
            curwin_mut().w_set_curswant.set(true);
        }
        c if c == K_KENTER || c == CAR => {
            if bt_quickfix(Some(&crate::globals::curbuf_mut())) {
                qf_view_result(true);
            }
        }
        c if c == b'g' as i32 || c == CTRL_G => {
            g_st().no_mapping.set(g_st().no_mapping.get() + (1));
            g_st().allow_keys.set(g_st().allow_keys.get() + (1));
            if xchar == NUL as i32 {
                xchar = plain_vgetc();
            }
            langmap_adjust(&mut xchar, true);
            g_st().no_mapping.set(g_st().no_mapping.get() - (1));
            g_st().allow_keys.set(g_st().allow_keys.get() - (1));
            add_to_showcmd(xchar);

            match xchar {
                xc if xc == b'}' as i32 || xc == b']' as i32 || xc == CTRL_RSB_CONST => {
                    if xchar == b'}' as i32 {
                        xchar = CTRL_RSB_CONST;
                        if prenum != 0 {
                            g_st().g_do_tagpreview.set(prenum);
                        } else {
                            g_st().g_do_tagpreview.set(opt_vars().p_pvh.get() as i32);
                        }
                    }
                    if prenum != 0 {
                        g_st().postponed_split.set(prenum);
                    } else {
                        g_st().postponed_split.set(-1);
                    }
                    do_nv_ident(b'g' as i32, xchar);
                    g_st().postponed_split.set(0);
                }
                xc if xc == b'f' as i32 || xc == b'F' as i32 => {
                    g_st().cmdmod.borrow_mut().cmod_tab =
                        tabpage_index(g_st().curtab.get().opt()) + 1;
                    nchar = xchar;
                    do_window_gotofile(nchar, prenum1);
                }
                xc if xc == b't' as i32 => {
                    goto_tabpage(prenum);
                }
                xc if xc == b'T' as i32 => {
                    goto_tabpage(-prenum1);
                }
                xc if xc == TAB => {
                    if !goto_tabpage_lastused() {
                        beep_flush();
                    }
                }
                xc if xc == b'e' as i32 => {
                    if curwin_mut().w_floating.get() || !ui_has(K_UIMULTIGRID) {
                        beep_flush();
                        return;
                    }
                    let mut config = WIN_CONFIG_INIT;
                    config.width = curwin_mut().w_width.get();
                    config.height = curwin_mut().w_height.get();
                    config.external = true;
                    let mut err = ERROR_INIT;
                    if win_new_float(Some(&curwin_mut()), false, config, &mut err).is_none() {
                        emsg(err.msg_cstr().unwrap_or(c""));
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

fn cmd_with_count(cmd: &std::ffi::CStr, cbuf: &mut [u8], prenum: i64) {
    let bufsize = cbuf.len();
    let len = crate::memory::copy_bytes_to_c_buf(cbuf, cmd.to_bytes());
    if prenum > 0 && len < bufsize {
        format_to_buf!(&mut cbuf[len..], c"%lld", prenum);
    }
}

pub fn win_set_buf(win: &WinInfoInfo, buf: &FileBuf, err: &mut Error) {
    let win_handle = win.handle;
    let tab = win_find_tabpage(win);

    g_st()
        .redrawing_disabled
        .set(g_st().redrawing_disabled.get() + (1));

    let mut switchwin: CtxSwitch = CtxSwitch::new();

    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    let win_ok = ctx_switch(
        &mut switchwin,
        Some(crate::globals::WinPtr::from(win).borrow_mut()),
        tab.as_deref(),
        None,
        K_CTX_NO_DISPLAY | K_CTX_KEEP_CWD,
    );
    if win_ok {
        let save_acd = opt_vars().p_acd.get();
        if !switchwin.cs_same_win {
            opt_vars().p_acd.set(false);
        }

        do_buffer(DOBUF_GOTO, DOBUF_FIRST, FORWARD, buf.b_fnum.get(), 0);

        if !switchwin.cs_same_win {
            opt_vars().p_acd.set(save_acd);
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

    validate_cursor(&curwin_mut());

    ctx_restore(&mut switchwin);
    g_st()
        .redrawing_disabled
        .set(g_st().redrawing_disabled.get() - (1));
}

/// Return the number of fold columns to display
pub fn win_fdccol_count(wp: &WinInfoInfo) -> i32 {
    let fdc_bytes = wp.w_onebuf_opt.wo_fdc.as_bytes();
    if fdc_bytes.starts_with(b"auto") {
        let fdccol = if fdc_bytes.get(4) == Some(&b':') {
            i32::from(fdc_bytes.get(5).copied().unwrap_or(0)) - i32::from(b'0')
        } else {
            1
        };
        let needed_fdccols = get_deepest_nesting(wp);
        return fdccol.min(needed_fdccols);
    }
    i32::from(fdc_bytes.first().copied().unwrap_or(0)) - i32::from(b'0')
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

pub fn ui_ext_win_position(wp: &WinInfoInfo, validate: bool) {
    wp.w_pos_changed.set(false);
    if !wp.w_floating.get() {
        if ui_has(K_UIMULTIGRID) {
            wp.w_grid_alloc.comp_col.set(wp.w_wincol.get());
            wp.w_grid_alloc.comp_row.set(wp.w_winrow.get());
        }
        ui_call_win_pos(
            Integer::from(wp.w_grid_alloc.handle.get()),
            wp.handle,
            Integer::from(wp.w_winrow.get()),
            Integer::from(wp.w_wincol.get()),
            Integer::from(wp.w_width.get()),
            Integer::from(wp.w_height.get()),
        );
        return;
    }

    let c = wp.w_config.borrow().clone();
    if !c.external {
        let mut grid_comp_row = g_st().default_grid.comp_row.get();
        let mut grid_comp_col = g_st().default_grid.comp_col.get();
        let mut grid_handle = g_st().default_grid.handle.get();
        let mut row = c.row;
        let mut col = c.col;
        if c.relative == K_FLOAT_RELATIVE_WINDOW {
            let mut dummy = ERROR_INIT;
            let win = find_window_by_handle(c.window, &mut dummy);
            api_clear_error(&mut dummy);
            if !win.is_null() {
                let win = win.borrow_mut();
                if win.w_pos_changed.get()
                    && !win.w_grid_alloc.chars.borrow().is_empty()
                    && win_valid(Some(win))
                {
                    ui_ext_win_position(win, validate);
                }
                let mut row_off: i32 = 0;
                let mut col_off: i32 = 0;
                win_grid_alloc(win);
                let g = grid_adjust(&win.w_grid, &mut row_off, &mut col_off);
                grid_comp_row = g.comp_row.get();
                grid_comp_col = g.comp_col.get();
                grid_handle = g.handle.get();
                row += f64::from(row_off);
                col += f64::from(col_off);
                if c.bufpos.lnum >= 0 {
                    let lnum = (c.bufpos.lnum + 1).min(win.buf_mut().b_ml.ml_line_count.get());
                    let mut pos = CursorPos {
                        lnum,
                        col: c.bufpos.col,
                        coladd: 0,
                    };
                    let mut trow: i32 = 0;
                    let mut tcol: i32 = 0;
                    let mut tcolc: i32 = 0;
                    let mut tcole: i32 = 0;
                    textpos2screenpos(
                        win, &mut pos, &mut trow, &mut tcol, &mut tcolc, &mut tcole, true,
                    );
                    row += f64::from(trow - 1);
                    col += f64::from(tcol - 1);
                }
            }
        } else if c.relative == K_FLOAT_RELATIVE_LASTSTATUS {
            row += f64::from(
                g_st().rows_global.get() - opt_vars().p_ch.get() as i32 - last_stl_height(false),
            );
        } else if c.relative == K_FLOAT_RELATIVE_TABLINE {
            row += f64::from(tabline_height());
        }

        let resort = wp.w_grid_alloc.comp_index.get() != 0
            && wp.w_grid_alloc.zindex.get() != wp.w_config.borrow().zindex;
        let raise = resort && wp.w_grid_alloc.zindex.get() < wp.w_config.borrow().zindex;
        wp.w_grid_alloc.zindex.set(wp.w_config.borrow().zindex);
        if resort {
            ui_comp_layers_adjust(wp.w_grid_alloc.comp_index.get(), raise);
        }
        let valid = wp.w_redr_type.get() == 0 || ui_has(K_UIMULTIGRID);
        if !valid && !validate {
            wp.w_pos_changed.set(true);
            return;
        }

        let east = (c.anchor & K_FLOAT_ANCHOR_EAST) != 0;
        let south = (c.anchor & K_FLOAT_ANCHOR_SOUTH) != 0;

        let mut comp_row = (row as i32) - if south { wp.w_height_outer.get() } else { 0 };
        let mut comp_col = (col as i32) - if east { wp.w_width_outer.get() } else { 0 };
        let above_ch = if wp.w_config.borrow().zindex < K_ZINDEX_MESSAGES {
            opt_vars().p_ch.get() as i32
        } else {
            0
        };
        comp_row += grid_comp_row;
        comp_col += grid_comp_col;
        comp_row = comp_row
            .min(g_st().rows_global.get() - wp.w_height_outer.get() - above_ch)
            .max(0);
        if !c.fixed || east {
            comp_col = comp_col
                .min(g_st().columns_global.get() - wp.w_width_outer.get())
                .max(0);
        }
        wp.w_winrow.set(comp_row);
        wp.w_wincol.set(comp_col);

        if !c.hide {
            ui_comp_put_grid(
                &wp.w_grid_alloc,
                comp_row,
                comp_col,
                wp.w_height_outer.get(),
                wp.w_width_outer.get(),
                valid,
                false,
            );
            if resort {
                ui_comp_compose_grid(&wp.w_grid_alloc);
            }
            if ui_has(K_UIMULTIGRID) {
                let anchor = cstr_as_string(Some(FLOAT_ANCHOR_STR[c.anchor as usize]));
                ui_call_win_float_pos(
                    Integer::from(wp.w_grid_alloc.handle.get()),
                    wp.handle,
                    anchor,
                    Integer::from(grid_handle),
                    row,
                    col,
                    c.mouse,
                    Integer::from(wp.w_grid_alloc.zindex.get()),
                    wp.w_grid_alloc.comp_index.get() as Integer,
                    Integer::from(wp.w_winrow.get()),
                    Integer::from(wp.w_wincol.get()),
                );
            }
            ui_check_cursor_grid(wp.w_grid_alloc.handle.get());
            let mouse_en = wp.w_config.borrow().mouse;
            wp.w_grid_alloc.mouse_enabled.set(mouse_en);
            if !valid {
                wp.w_grid_alloc.valid.set(false);
                redraw_later(Some(wp), UPD_NOT_VALID);
            }
        } else {
            if ui_has(K_UIMULTIGRID) {
                ui_call_win_hide(Integer::from(wp.w_grid_alloc.handle.get()));
            }
            ui_comp_remove_grid(&wp.w_grid_alloc);
        }
    } else {
        ui_call_win_external_pos(Integer::from(wp.w_grid_alloc.handle.get()), wp.handle);
    }
}

pub fn ui_ext_win_viewport(wp: &WinInfoInfo) {
    if (wp == curwin_ptr() || ui_has(K_UIMULTIGRID))
        && wp.w_viewport_invalid.get()
        && wp.w_redr_type.get() == 0
    {
        let line_count = wp.buf_mut().b_ml.ml_line_count.get();
        let cur_topline = wp.w_topline.get().min(line_count);
        let cur_botline = wp.w_botline.get().min(line_count);
        let mut delta: i64 = 0;
        let mut last_topline = wp.w_viewport_last_topline.get();
        let mut last_botline = wp.w_viewport_last_botline.get();
        let mut last_topfill = wp.w_viewport_last_topfill.get();
        let mut last_skipcol = i64::from(wp.w_viewport_last_skipcol.get());
        if last_topline > line_count {
            delta -= i64::from(last_topline - line_count);
            last_topline = line_count;
            last_topfill = 0;
            last_skipcol = i64::from(MAXCOL);
        }
        last_botline = last_botline.min(line_count);
        if cur_topline < last_topline
            || (cur_topline == last_topline && i64::from(wp.w_skipcol.get()) < last_skipcol)
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
                i64::from(wp.w_skipcol.get()),
                &mut lnume,
                &mut vcole,
                None,
                i64::MAX,
                true,
            );
        } else if cur_topline > last_topline
            || (cur_topline == last_topline && i64::from(wp.w_skipcol.get()) > last_skipcol)
        {
            let mut vcole = i64::from(wp.w_skipcol.get());
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
                true,
            );
        }
        delta += i64::from(last_topfill);
        delta -= i64::from(wp.w_topfill.get());
        let mut ev_botline = wp.w_botline.get();
        if ev_botline == line_count + 1 && wp.w_empty_rows.get() == 0 {
            ev_botline = line_count;
        }
        ui_call_win_viewport(
            Integer::from(wp.w_grid_alloc.handle.get()),
            wp.handle,
            Integer::from(wp.w_topline.get() - 1),
            Integer::from(ev_botline),
            Integer::from(wp.w_cursor.get().lnum - 1),
            Integer::from(wp.w_cursor.get().col),
            Integer::from(line_count),
            delta,
        );
        wp.w_viewport_invalid.set(false);
        wp.w_viewport_last_topline.set(wp.w_topline.get());
        wp.w_viewport_last_botline.set(wp.w_botline.get());
        wp.w_viewport_last_topfill.set(wp.w_topfill.get());
        wp.w_viewport_last_skipcol.set(wp.w_skipcol.get());
    }
}

/// If "split_disallowed" is set, or "wp"'s buffer is closing, give an error and return FAIL.
/// Otherwise return OK.
pub fn check_split_disallowed(wp: Option<&WinInfoInfo>) -> bool {
    let mut err = ERROR_INIT;
    let ok = check_split_disallowed_err(wp, &mut err);
    if error_set(&err) {
        emsg(gettext((err.msg_cstr()).unwrap_or(c"")));
        api_clear_error(&mut err);
    }
    ok
}

/// Like `check_split_disallowed`, but set `err` to the (untranslated) error message on failure and
/// return false. Otherwise return true.
pub fn check_split_disallowed_err(wp: Option<&WinInfoInfo>, err: &mut Error) -> bool {
    if WIN_STATE.with(|s| s.split_disallowed.get()) > 0 {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"E242: Can't split a window while closing another",
        );
        return false;
    }
    if wp.is_some_and(|w| w.buf_mut().b_locked_split.get() != 0) {
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

pub fn win_split(size: i32, mut flags: i32) -> bool {
    if !check_split_disallowed(curwin_ptr().opt()) {
        return false;
    }

    if may_open_tabpage() {
        return true;
    }

    flags |= g_st().cmdmod.borrow().cmod_split;
    if (flags & WSP_TOP) != 0 && (flags & WSP_BOT) != 0 {
        emsg(gettext(
            c"E442: Can't split topleft and botright at the same time",
        ));
        return false;
    }

    if (flags & WSP_HELP) != 0 {
        make_snapshot(SNAP_HELP_IDX as i32);
    } else {
        clear_snapshot(&curtab_mut(), SNAP_HELP_IDX as i32);
    }

    if (flags & WSP_QUICKFIX) != 0 {
        make_snapshot(SNAP_QUICKFIX_IDX as i32);
    } else {
        clear_snapshot(&curtab_mut(), SNAP_QUICKFIX_IDX as i32);
    }

    win_split_ins(size, flags, None, 0, None).is_some()
}

pub fn win_split_ins(
    size: i32,
    flags: i32,
    new_wp_in: Option<&WinInfoInfo>,
    dir: i32,
    to_flatten_in: Option<&Frame>,
) -> Option<crate::globals::WinPtr> {
    let new_wp = crate::globals::WinPtr::from(new_wp_in);
    let to_flatten = crate::globals::FramePtr::from(to_flatten_in);
    let mut wp = new_wp.clone();

    if let Some(new_wp) = new_wp.opt()
        && is_ctx_win(new_wp)
    {
        return None;
    }

    if new_wp.is_null() {
        trigger_winnewpre();
    }

    let oldwin = if (flags & WSP_TOP) != 0 {
        g_st().firstwin.get()
    } else if (flags & WSP_BOT) != 0 || curwin_mut().w_floating.get() {
        lastwin_nofloating(None)
    } else {
        curwin_ptr()
    };
    let oldwin = oldwin.borrow_mut();

    let mut need_status: i32 = 0;
    let mut new_size = size;
    let vertical = (flags & WSP_VERT) != 0;
    let toplevel = (flags & (WSP_TOP | WSP_BOT)) != 0;

    if one_window(&firstwin_mut(), None)
        && opt_vars().p_ls.get() == 1
        && oldwin.w_status_height.get() == 0
    {
        if OptInt::from(oldwin.w_height.get()) <= opt_vars().p_wmh.get() {
            emsg(gettext(E_NOROOM));
            return None;
        }
        need_status = STATUS_HEIGHT;
        win_float_anchor_laststatus();
    }

    let mut do_equal = false;
    let mut oldwin_height: i32 = 0;
    let layout = if vertical { FR_ROW } else { FR_COL };
    let mut did_set_fraction = false;

    if vertical {
        let wmw1 = if opt_vars().p_wmw.get() == 0 {
            1
        } else {
            opt_vars().p_wmw.get() as i32
        };
        let mut needed = wmw1 + 1;
        if (flags & WSP_ROOM) != 0 {
            needed += opt_vars().p_wiw.get() as i32 - wmw1;
        }
        let mut minwidth: i32;
        let available: i32;
        if toplevel {
            minwidth = frame_minwidth(&topframe_mut(), nowin());
            available = crate::globals::topframe_mut().fr_width.get();
            needed += minwidth;
        } else if opt_vars().p_ea.get() {
            minwidth = frame_minwidth(oldwin.w_frame.get().borrow(), nowin());

            let mut prevfrp = oldwin.w_frame.get().clone();
            let mut frp = oldwin.frame_mut().fr_parent.get();
            while !frp.is_null() {
                if frp.fr_layout.get() == FR_ROW {
                    for frp2 in frame_iter_ptr(frp.fr_child.get()) {
                        if prevfrp != frp2 {
                            minwidth += frame_minwidth(&frp2, nowin());
                        }
                    }
                }
                prevfrp = frp.clone();

                frp = frp.fr_parent.get();
            }
            available = crate::globals::topframe_mut().fr_width.get();
            needed += minwidth;
        } else {
            minwidth = frame_minwidth(oldwin.w_frame.get().borrow(), nowin());

            available = oldwin.frame_mut().fr_width.get();
            needed += minwidth;
        }
        if available < needed {
            emsg(gettext(E_NOROOM));
            return None;
        }
        if new_size == 0 {
            new_size = oldwin.w_width.get() / 2;
        }
        new_size = new_size.min(available - minwidth - 1).max(wmw1);

        if OptInt::from(oldwin.w_width.get() - new_size - 1) < opt_vars().p_wmw.get() {
            do_equal = true;
        }

        if oldwin.w_onebuf_opt.wo_wfw.get() {
            win_setwidth_win(oldwin.w_width.get() + new_size + 1, oldwin, true);
        }

        if !do_equal
            && opt_vars().p_ea.get()
            && size == 0
            && opt_vars().p_ead.first_byte() != b'v'
            && !oldwin.frame_mut().fr_parent.get().is_null()
        {
            for frp in frame_iter_mut(oldwin.frame_mut().fr_parent.get().fr_child.get()) {
                if frp.fr_win.get() != oldwin
                    && !frp.fr_win.get().is_null()
                    && (frp.win_mut().w_width.get() > new_size
                        || frp.win_mut().w_width.get() > (oldwin.w_width.get() - new_size - 1))
                {
                    do_equal = true;
                    break;
                }
            }
        }
    } else {
        let wmh1 = (opt_vars().p_wmh.get() as i32).max(1) + oldwin.w_winbar_height.get();
        let mut needed = wmh1 + STATUS_HEIGHT;
        if (flags & WSP_ROOM) != 0 {
            needed += opt_vars().p_wh.get() as i32 - wmh1 + oldwin.w_winbar_height.get();
        }
        if opt_vars().p_ch.get() < 1 {
            needed += 1;
        }
        let mut minheight: i32;
        let available: i32;
        if toplevel {
            minheight = frame_minheight(&topframe_mut(), nowin()) + need_status;
            available = crate::globals::topframe_mut().fr_height.get();
            needed += minheight;
        } else if opt_vars().p_ea.get() {
            minheight = frame_minheight(oldwin.w_frame.get().borrow(), nowin()) + need_status;

            let mut prevfrp = oldwin.w_frame.get().clone();
            let mut frp = oldwin.frame_mut().fr_parent.get();
            while !frp.is_null() {
                if frp.fr_layout.get() == FR_COL {
                    for frp2 in frame_iter_ptr(frp.fr_child.get()) {
                        if prevfrp != frp2 {
                            minheight += frame_minheight(&frp2, nowin());
                        }
                    }
                }
                prevfrp = frp.clone();

                frp = frp.fr_parent.get();
            }
            available = crate::globals::topframe_mut().fr_height.get();
            needed += minheight;
        } else {
            minheight = frame_minheight(oldwin.w_frame.get().borrow(), nowin()) + need_status;

            available = oldwin.frame_mut().fr_height.get();
            needed += minheight;
        }
        if available < needed {
            emsg(gettext(E_NOROOM));
            return None;
        }
        oldwin_height = oldwin.w_height.get();
        if need_status != 0 {
            oldwin.w_status_height.set(STATUS_HEIGHT);
            oldwin_height -= STATUS_HEIGHT;
        }
        if new_size == 0 {
            new_size = oldwin_height / 2;
        }

        new_size = new_size
            .min(available - minheight - STATUS_HEIGHT)
            .max(wmh1);

        if OptInt::from(oldwin_height - new_size - STATUS_HEIGHT) < opt_vars().p_wmh.get() {
            do_equal = true;
        }

        if oldwin.w_onebuf_opt.wo_wfh.get() {
            set_fraction(oldwin);
            did_set_fraction = true;

            win_setheight_win(
                oldwin.w_height.get() + new_size + STATUS_HEIGHT,
                oldwin,
                true,
            );
            oldwin_height = oldwin.w_height.get();
        }

        if !do_equal
            && opt_vars().p_ea.get()
            && size == 0
            && opt_vars().p_ead.first_byte() != b'h'
            && !oldwin.frame_mut().fr_parent.get().is_null()
        {
            for frp in frame_iter_mut(oldwin.frame_mut().fr_parent.get().fr_child.get()) {
                if frp.fr_win.get() != oldwin
                    && !frp.fr_win.get().is_null()
                    && (frp.win_mut().w_height.get() > new_size
                        || frp.win_mut().w_height.get() > oldwin_height - new_size - STATUS_HEIGHT)
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
                    opt_vars().p_spr.get()
                } else {
                    opt_vars().p_sb.get()
                }))
    {
        if new_wp.is_null() {
            wp = win_alloc(Some(oldwin), false);
        } else {
            win_append(Some(oldwin), wp.borrow_mut(), None);
        }
    } else if new_wp.is_null() {
        wp = win_alloc(oldwin.w_prev.get().opt_mut(), false);
    } else {
        win_append(oldwin.w_prev.get().opt_mut(), wp.borrow_mut(), None);
    }

    if new_wp.is_null() && wp.is_null() {
        return None;
    }
    let wp = wp.borrow_mut();

    if new_wp.is_null() {
        new_frame(wp);
        win_init(wp, &curwin_mut(), flags);
    } else if wp.w_floating.get() {
        ui_comp_remove_grid(&wp.w_grid_alloc);
        if ui_has(K_UIMULTIGRID) {
            wp.w_pos_changed.set(true);
        } else {
            ui_call_win_hide(Integer::from(wp.w_grid_alloc.handle.get()));
            win_free_grid(wp, true);
        }

        if wp.w_config.borrow().external {
            for tp in for_all_tabpages_ptr() {
                if g_st().curtab.get() != tp && tp.tp_curwin.get() == wp {
                    tp.tp_curwin.set(tp.tp_firstwin.get());
                }
            }
        }

        wp.w_floating.set(false);
        new_frame(wp);

        clear_float_config(&mut wp.w_config.borrow_mut(), true);
        for c in &wp.w_border_adj {
            c.set(0);
        }
    }

    if let Some(tf) = to_flatten.opt() {
        frame_flatten(tf);
    }

    let before: bool;
    let mut curfrp: crate::globals::FramePtr;

    if toplevel {
        if (crate::globals::topframe_mut().fr_layout.get() == FR_COL && !vertical)
            || (crate::globals::topframe_mut().fr_layout.get() == FR_ROW && vertical)
        {
            curfrp = crate::globals::topframe_mut().fr_child.get();
            if (flags & WSP_BOT) != 0 {
                while !curfrp.fr_next.get().is_null() {
                    curfrp = curfrp.fr_next.get();
                }
            }
        } else {
            curfrp = g_st().topframe.get().clone();
        }
        before = (flags & WSP_TOP) != 0;
    } else {
        curfrp = oldwin.w_frame.get().clone();
        if (flags & WSP_BELOW) != 0 {
            before = false;
        } else if (flags & WSP_ABOVE) != 0 {
            before = true;
        } else if vertical {
            before = !opt_vars().p_spr.get();
        } else {
            before = !opt_vars().p_sb.get();
        }
    }
    if curfrp.fr_parent.get().is_null() || curfrp.parent_mut().fr_layout.get() != layout {
        let mut frp = crate::globals::FramePtr::from(Frame::new_rc_from(&curfrp));
        frp.fr_parent.set(curfrp.clone());
        frp.fr_next.set(crate::globals::FramePtr::NULL);
        frp.fr_prev.set(crate::globals::FramePtr::NULL);

        curfrp.fr_layout.set(layout);
        curfrp.fr_child.set(frp.clone());
        curfrp.fr_win.set(crate::globals::WinPtr::NULL);

        curfrp = frp.clone();
        if !frp.fr_win.get().is_null() {
            oldwin.w_frame.set(frp);
        } else {
            frp = frp.fr_child.get();

            while !frp.is_null() {
                frp.fr_parent.set(curfrp.clone());
                frp = frp.fr_next.get();
            }
        }
    }

    let frp = if new_wp.is_null() {
        wp.w_frame.get().clone()
    } else {
        new_wp.borrow_mut().w_frame.get().clone()
    };

    frp.fr_parent.set(curfrp.fr_parent.get());

    if before {
        frame_insert(curfrp.borrow(), &frp);
    } else {
        frame_append(curfrp.borrow(), &frp);
    }

    if !did_set_fraction {
        set_fraction(oldwin);
    }
    wp.w_fraction.set(oldwin.w_fraction.get());

    if vertical {
        wp.w_onebuf_opt
            .wo_scr
            .set(curwin_mut().w_onebuf_opt.wo_scr.get());

        if need_status != 0 {
            win_new_height(oldwin, oldwin.w_height.get() - 1);
            oldwin.w_status_height.set(need_status);
        }
        if toplevel {
            wp.w_winrow.set(tabline_height());
            let ls_h = i32::from(opt_vars().p_ls.get() == 1 || opt_vars().p_ls.get() == 2);

            win_new_height(wp, curfrp.fr_height.get() - ls_h);

            wp.w_status_height.set(ls_h);
            wp.w_hsep_height.set(0);
        } else {
            wp.w_winrow.set(oldwin.w_winrow.get());
            win_new_height(wp, oldwin.w_height.get());
            wp.w_status_height.set(oldwin.w_status_height.get());
            wp.w_hsep_height.set(oldwin.w_hsep_height.get());
        }

        frp.fr_height.set(curfrp.fr_height.get());

        win_new_width(wp, new_size);
        if before {
            wp.w_vsep_width.set(1);
        } else {
            wp.w_vsep_width.set(oldwin.w_vsep_width.get());
            oldwin.w_vsep_width.set(1);
        }
        if toplevel {
            if (flags & WSP_BOT) != 0 {
                frame_set_vsep(curfrp.borrow(), true);
            }

            let cur_w = curfrp.fr_width.get();
            frame_new_width(
                curfrp.borrow(),
                cur_w - (new_size + i32::from((flags & WSP_TOP) != 0)),
                (flags & WSP_TOP) != 0,
                false,
            );
        } else {
            win_new_width(oldwin, oldwin.w_width.get() - (new_size + 1));
        }
        if before {
            wp.w_wincol.set(oldwin.w_wincol.get());
            oldwin.w_wincol.set(oldwin.w_wincol.get() + (new_size + 1));
        } else {
            wp.w_wincol
                .set(oldwin.w_wincol.get() + oldwin.w_width.get() + 1);
        }
        frame_fix_width(oldwin);
        frame_fix_width(wp);
    } else {
        let is_stl_global = global_stl_height() > 0;
        if toplevel {
            wp.w_wincol.set(0);
            win_new_width(wp, g_st().columns_global.get());
            wp.w_vsep_width.set(0);
        } else {
            wp.w_wincol.set(oldwin.w_wincol.get());
            win_new_width(wp, oldwin.w_width.get());
            wp.w_vsep_width.set(oldwin.w_vsep_width.get());
        }

        frp.fr_width.set(curfrp.fr_width.get());

        win_new_height(wp, new_size);
        let old_status_height = oldwin.w_status_height.get();
        if before {
            wp.w_hsep_height.set(i32::from(is_stl_global));
        } else {
            wp.w_hsep_height.set(oldwin.w_hsep_height.get());
            oldwin.w_hsep_height.set(i32::from(is_stl_global));
        }
        if toplevel {
            let mut new_fr_height = curfrp.fr_height.get() - new_size;
            if is_stl_global {
                if (flags & WSP_BOT) != 0 {
                    frame_add_hsep(curfrp.borrow());
                } else {
                    new_fr_height -= 1;
                }
            } else {
                if !((flags & WSP_BOT) != 0 && opt_vars().p_ls.get() == 0) {
                    new_fr_height -= STATUS_HEIGHT;
                }
                if (flags & WSP_BOT) != 0 {
                    frame_add_statusline(curfrp.borrow());
                }
            }

            frame_new_height(
                curfrp.borrow(),
                new_fr_height,
                (flags & WSP_TOP) != 0,
                false,
                false,
            );
        } else {
            win_new_height(oldwin, oldwin_height - (new_size + STATUS_HEIGHT));
        }

        if before {
            wp.w_winrow.set(oldwin.w_winrow.get());
            if is_stl_global {
                wp.w_status_height.set(0);
                oldwin
                    .w_winrow
                    .set(oldwin.w_winrow.get() + (wp.w_height.get() + 1));
            } else {
                wp.w_status_height.set(STATUS_HEIGHT);
                oldwin
                    .w_winrow
                    .set(oldwin.w_winrow.get() + (wp.w_height.get() + STATUS_HEIGHT));
            }
        } else if is_stl_global {
            wp.w_winrow
                .set(oldwin.w_winrow.get() + oldwin.w_height.get() + 1);
            wp.w_status_height.set(0);
        } else {
            wp.w_winrow
                .set(oldwin.w_winrow.get() + oldwin.w_height.get() + STATUS_HEIGHT);
            wp.w_status_height.set(old_status_height);
            if (flags & WSP_BOT) == 0 {
                oldwin.w_status_height.set(STATUS_HEIGHT);
            }
        }
        frame_fix_height(wp);
        frame_fix_height(oldwin);
    }

    if toplevel {
        win_comp_pos();
    }

    redraw_later(Some(wp), UPD_NOT_VALID);
    redraw_later(Some(oldwin), UPD_NOT_VALID);
    status_redraw_all();

    if need_status != 0 {
        g_st().msg_row.set(g_st().rows_global.get() - 1);
        g_st().msg_col.set(g_st().sc_col.get());
        msg_clr_eos_force();
        comp_col();
        g_st().msg_row.set(g_st().rows_global.get() - 1);
        g_st().msg_col.set(0);
    }

    if do_equal || dir != 0 {
        win_equal(
            Some(wp),
            true,
            if vertical {
                if dir == b'v' as i32 {
                    b'b' as i32
                } else {
                    b'h' as i32
                }
            } else if dir == b'h' as i32 {
                b'b' as i32
            } else {
                b'v' as i32
            },
        );
    } else if !is_ctx_win(wp) {
        win_fix_scroll(false);
    }

    let i: i32;
    if (flags & WSP_VERT) != 0 {
        i = opt_vars().p_wiw.get() as i32;
        if size != 0 {
            opt_vars().p_wiw.set(OptInt::from(size));
        }
    } else {
        i = opt_vars().p_wh.get() as i32;
        if size != 0 {
            opt_vars().p_wh.set(OptInt::from(size));
        }
    }

    let oldwin_ptr = crate::globals::WinPtr::from(oldwin);
    let wp_ptr = crate::globals::WinPtr::from(wp);
    if (flags & WSP_NOENTER) == 0 {
        win_enter_ext(
            wp_ptr.borrow_mut(),
            (if new_wp.is_null() {
                WEE_TRIGGER_NEW_AUTOCMDS
            } else {
                0
            }) | WEE_TRIGGER_ENTER_AUTOCMDS
                | WEE_TRIGGER_LEAVE_AUTOCMDS,
        );
    }
    if vertical {
        opt_vars().p_wiw.set(OptInt::from(i));
    } else {
        opt_vars().p_wh.set(OptInt::from(i));
    }

    if win_valid(oldwin_ptr.opt()) {
        oldwin_ptr.w_pos_changed.set(true);
    }

    (!wp_ptr.is_null()).then_some(wp_ptr)
}
pub fn win_init(newp: &WinInfoInfo, oldp: &WinInfoInfo, flags: i32) {
    newp.w_buffer.set(oldp.w_buffer.get());
    newp.w_s_owned.set(None);
    oldp.buf_mut()
        .b_nwindows
        .set(oldp.buf_mut().b_nwindows.get() + (1));
    newp.w_cursor.set(oldp.w_cursor.get());
    newp.w_valid.set(0);
    newp.w_curswant.set(oldp.w_curswant.get());
    newp.w_set_curswant.set(oldp.w_set_curswant.get());
    newp.w_topline.set(oldp.w_topline.get());
    newp.w_topfill.set(oldp.w_topfill.get());
    newp.w_leftcol.set(oldp.w_leftcol.get());
    newp.w_pcmark.set(oldp.w_pcmark.get());
    newp.w_prev_pcmark.set(oldp.w_prev_pcmark.get());
    newp.w_alt_fnum.set(oldp.w_alt_fnum.get());
    newp.w_wrow.set(oldp.w_wrow.get());
    newp.w_fraction.set(oldp.w_fraction.get());
    newp.w_prev_fraction_row.set(oldp.w_prev_fraction_row.get());
    copy_jumplist(oldp, newp);
    if (flags & WSP_NEWLOC) != 0 {
        newp.w_llist.set(crate::quickfix::QfInfoId::NULL);
        newp.w_llist_ref.set(crate::quickfix::QfInfoId::NULL);
    } else {
        copy_loclist_stack(oldp, newp);
    }
    newp.w_localdir.set(oldp.w_localdir.get().clone());
    newp.w_prevdir.set(oldp.w_prevdir.get().clone());

    if opt_vars().p_spk.first_byte() != b'c' {
        if opt_vars().p_spk.first_byte() == b't' {
            newp.w_skipcol.set(oldp.w_skipcol.get());
        }
        newp.w_botline.set(oldp.w_botline.get());
        newp.w_prev_height.set(oldp.w_height.get());
        newp.w_prev_winrow.set(oldp.w_winrow.get());
    }

    for i in 0..oldp.w_tagstacklen.get() as usize {
        newp.w_tagstack.borrow_mut()[i] = oldp.w_tagstack.borrow()[i].clone();
    }
    newp.w_tagstackidx.set(oldp.w_tagstackidx.get());
    newp.w_tagstacklen.set(oldp.w_tagstacklen.get());

    newp.w_changelistidx.set(oldp.w_changelistidx.get());

    copy_folding_state(oldp, newp);

    win_init_some(newp, oldp);

    newp.w_winbar_height.set(oldp.w_winbar_height.get());
}

fn win_init_some(newp: &WinInfoInfo, oldp: &WinInfoInfo) {
    newp.w_alist.set(oldp.w_alist.get().clone());
    newp.w_arg_idx.set(oldp.w_arg_idx.get());

    win_copy_options(oldp, newp);
}

pub fn win_valid(win: Option<&WinInfoInfo>) -> bool {
    tabpage_win_valid(g_st().curtab.get().opt(), win)
}

pub fn tabpage_win_valid(tp: Option<&TabPageInfo>, win: Option<&WinInfoInfo>) -> bool {
    let (Some(tp_ref), Some(win_ref)) = (tp, win) else {
        return false;
    };
    let win_ptr = WinPtr::from(win_ref);
    let mut wp = if g_st().curtab.get() == tp_ref {
        g_st().firstwin.get()
    } else {
        tp_ref.tp_firstwin.get()
    };
    while let Some(w) = wp.opt() {
        if wp == win_ptr {
            return true;
        }
        wp = w.w_next.get();
    }
    false
}

#[expect(clippy::manual_find)]
pub fn win_find_by_handle(handle: ObjHandle) -> Option<crate::globals::WinPtr> {
    for wp in for_all_windows_ptr() {
        if wp.handle == handle {
            return Some(wp);
        }
    }
    None
}

pub fn win_valid_any_tab(win: Option<&WinInfoInfo>) -> bool {
    let mut tp = g_st().first_tabpage.get().clone();
    while let Some(t) = tp.opt() {
        if tabpage_win_valid(Some(t), win) {
            return true;
        }
        tp = t.tp_next.get();
    }
    false
}

pub fn win_count() -> i32 {
    let mut count: i32 = 0;
    for _wp in for_all_windows_mut() {
        count += 1;
    }
    count
}

pub fn make_windows(mut count: i32, vertical: bool) -> i32 {
    let mut maxcount = if vertical {
        ((OptInt::from(curwin_mut().w_width.get() + curwin_mut().w_vsep_width.get())
            - (opt_vars().p_wiw.get() - opt_vars().p_wmw.get())) as i32)
            / (opt_vars().p_wmw.get() as i32 + 1)
    } else {
        ((OptInt::from(
            curwin_mut().w_height.get()
                + curwin_mut().w_hsep_height.get()
                + curwin_mut().w_status_height.get(),
        ) - (opt_vars().p_wh.get() - opt_vars().p_wmh.get())) as i32)
            / (opt_vars().p_wmh.get() as i32 + STATUS_HEIGHT + global_winbar_height())
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
                curwin_mut().w_width.get() - (curwin_mut().w_width.get() - todo) / (todo + 1) - 1,
                WSP_VERT | WSP_ABOVE,
            ) {
                break;
            }
        } else if !win_split(
            curwin_mut().w_height.get()
                - (curwin_mut().w_height.get() - todo * STATUS_HEIGHT) / (todo + 1)
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

fn win_exchange(mut prenum: i32) {
    if curwin_mut().w_floating.get() {
        emsg(E_FLOATEXCHANGE);
        return;
    }

    if one_window(&curwin_mut(), None) {
        beep_flush();
        return;
    }
    if text_or_buf_locked() {
        beep_flush();
        return;
    }

    let mut frp: crate::globals::FramePtr;
    if prenum != 0 {
        frp = curwin_mut().frame_mut().fr_parent.get().fr_child.get();

        while !frp.is_null() && {
            prenum -= 1;
            prenum > 0
        } {
            frp = frp.fr_next.get();
        }
    } else if !curwin_mut().frame_mut().fr_next.get().is_null() {
        frp = curwin_mut().frame_mut().fr_next.get();
    } else {
        frp = curwin_mut().frame_mut().fr_prev.get();
    }

    if frp.is_null() || frp.fr_win.get().is_null() || frp.fr_win.get() == curwin_ptr() {
        return;
    }
    let wp = frp.win_mut();

    let wp2 = curwin_mut().w_prev.get();
    let frp2 = curwin_mut().frame_mut().fr_prev.get();
    if wp.w_prev.get() != curwin_ptr() {
        win_remove(&curwin_mut(), None);

        frame_remove(curwin_mut().w_frame.get().borrow());

        win_append(wp.w_prev.get().opt_mut(), &curwin_mut(), None);

        frame_insert(frp.borrow(), curwin_mut().w_frame.get().borrow());
    }
    if wp2 != wp {
        win_remove(&wp, None);

        frame_remove(wp.w_frame.get().borrow());

        win_append(wp2.opt_mut(), &wp, None);
        if frp2.is_null() {
            frame_insert(
                wp.frame_mut().fr_parent.get().fr_child.get().borrow(),
                wp.w_frame.get().borrow(),
            );
        } else {
            frame_append(frp2.borrow(), wp.w_frame.get().borrow());
        }
    }
    curwin_mut().w_status_height.swap(&wp.w_status_height);
    curwin_mut().w_vsep_width.swap(&wp.w_vsep_width);
    curwin_mut().w_hsep_height.swap(&wp.w_hsep_height);

    frame_fix_height(&curwin_mut());
    frame_fix_height(&wp);
    frame_fix_width(&curwin_mut());
    frame_fix_width(&wp);

    win_comp_pos();

    if g_st().curbuf.get() != wp.w_buffer.get() {
        reset_visual_and_resel();
    } else if g_st().visual_global.borrow().active {
        wp.w_cursor.set(curwin_mut().w_cursor.get());
    }

    win_enter(&wp, true);
    redraw_later(Some(&crate::globals::curwin_mut()), UPD_NOT_VALID);
}

fn win_rotate(upwards: bool, mut count: i32) {
    if curwin_mut().w_floating.get() {
        emsg(E_FLOATEXCHANGE);
        return;
    }

    if count <= 0 || one_window(&curwin_mut(), None) {
        beep_flush();
        return;
    }

    let mut frp = curwin_mut().frame_mut().fr_parent.get().fr_child.get();
    while !frp.is_null() {
        if frp.fr_win.get().is_null() {
            emsg(gettext(c"E443: Cannot rotate when another window is split"));
            return;
        }

        frp = frp.fr_next.get();
    }

    let mut wp1 = crate::globals::WinPtr::NULL;
    let mut wp2 = crate::globals::WinPtr::NULL;

    while count > 0 {
        count -= 1;
        if upwards {
            frp = curwin_mut().frame_mut().fr_parent.get().fr_child.get();

            debug_assert!(!frp.is_null());

            wp1 = frp.fr_win.get();

            win_remove(wp1.borrow_mut(), None);

            frame_remove(frp.borrow());

            debug_assert!(!frp.fr_parent.get().fr_child.get().is_null());

            while !frp.fr_next.get().is_null() {
                frp = frp.fr_next.get();
            }

            win_append(frp.fr_win.get().opt_mut(), wp1.borrow_mut(), None);

            frame_append(frp.borrow(), wp1.w_frame.get().borrow());

            wp2 = frp.fr_win.get();
        } else {
            frp = curwin_mut().w_frame.get().clone();
            while !frp.fr_next.get().is_null() {
                frp = frp.fr_next.get();
            }

            wp1 = frp.fr_win.get();
            wp2 = wp1.w_prev.get();

            win_remove(wp1.borrow_mut(), None);

            frame_remove(frp.borrow());

            debug_assert!(!frp.fr_parent.get().fr_child.get().is_null());

            let after_win = frp
                .fr_parent
                .get()
                .fr_child
                .get()
                .fr_win
                .get()
                .opt()
                .map(|w| w.w_prev.get())
                .unwrap_or_default();
            win_append(after_win.opt_mut(), wp1.borrow_mut(), None);

            frame_insert(frp.fr_parent.get().fr_child.get().borrow(), frp.borrow());
        }

        wp2.w_status_height.swap(&wp1.w_status_height);
        wp2.w_hsep_height.swap(&wp1.w_hsep_height);

        frame_fix_height(wp1.borrow_mut());

        frame_fix_height(wp2.borrow_mut());

        wp2.w_vsep_width.swap(&wp1.w_vsep_width);

        frame_fix_width(wp1.borrow_mut());

        frame_fix_width(wp2.borrow_mut());

        win_comp_pos();
    }

    (wp1.borrow_mut()).w_pos_changed.set(true);

    (wp2.borrow_mut()).w_pos_changed.set(true);

    redraw_all_later(UPD_NOT_VALID);
}

pub fn win_splitmove(wp_in: &WinInfoInfo, size: i32, flags: i32) -> bool {
    let wp = crate::globals::WinPtr::from(wp_in);
    let mut dir: i32 = 0;
    let height = wp.borrow_mut().w_height.get();

    if one_window(wp.borrow_mut(), None) {
        return true;
    }
    if is_ctx_win(wp.borrow()) || !check_split_disallowed(wp.opt()) {
        return false;
    }

    let mut unflat_altfr = crate::globals::FramePtr::NULL;
    if wp.borrow_mut().w_floating.get() {
        win_remove(wp.borrow_mut(), None);
    } else {
        winframe_remove(wp.borrow_mut(), &mut dir, None, Some(&mut unflat_altfr));

        debug_assert!(!unflat_altfr.is_null());

        win_remove(wp.borrow_mut(), None);

        last_status(false);
        win_comp_pos();
    }

    if win_split_ins(size, flags, wp.opt_mut(), dir, unflat_altfr.opt()).is_none() {
        if !wp.borrow_mut().w_floating.get() {
            debug_assert!(!unflat_altfr.is_null());

            winframe_restore(wp.borrow_mut(), dir, unflat_altfr.borrow());
        }

        win_append(wp.w_prev.get().opt_mut(), wp.borrow_mut(), None);

        return false;
    }

    if size == 0
        && (flags & WSP_VERT) == 0
        && win_valid(wp.opt())
        && !wp.borrow_mut().w_floating.get()
    {
        win_setheight_win(height, wp.borrow_mut(), true);

        if opt_vars().p_ea.get() {
            win_equal(curwin_ptr().opt(), wp == curwin_ptr(), b'v' as i32);
        }
    }

    true
}

pub fn win_move_after(win1: &WinInfoInfo, win2: &WinInfoInfo) {
    if ptr::eq(win1, win2) {
        return;
    }

    if win2.w_next.get() != win1 {
        if win1.frame_mut().fr_parent.get() != win2.frame_mut().fr_parent.get() {
            iemsg(c"INTERNAL: trying to move a window into another frame");
            return;
        }

        if g_st().lastwin.get() == win1 {
            let prev = win1.prev_mut();
            prev.w_status_height.swap(&win1.w_status_height);
            prev.w_hsep_height.swap(&win1.w_hsep_height);

            if prev.w_vsep_width.get() == 1 {
                prev.w_vsep_width.set(0);
                prev.frame_mut()
                    .fr_width
                    .set(prev.frame_mut().fr_width.get() - 1);
                win1.w_vsep_width.set(1);
                win1.frame_mut()
                    .fr_width
                    .set(win1.frame_mut().fr_width.get() + 1);
            }
        } else if g_st().lastwin.get() == win2 {
            win1.w_status_height.swap(&win2.w_status_height);
            win1.w_hsep_height.swap(&win2.w_hsep_height);

            if win1.w_vsep_width.get() == 1 {
                win2.w_vsep_width.set(1);
                win2.frame_mut()
                    .fr_width
                    .set(win2.frame_mut().fr_width.get() + 1);
                win1.w_vsep_width.set(0);
                win1.frame_mut()
                    .fr_width
                    .set(win1.frame_mut().fr_width.get() - 1);
            }
        }
        win_remove(win1, None);
        frame_remove(&win1.frame_mut());
        win_append(Some(win2), win1, None);
        frame_append(&win2.frame_mut(), &win1.frame_mut());

        win_comp_pos();
        redraw_later(Some(&crate::globals::curwin_mut()), UPD_NOT_VALID);
    }
    win1.w_pos_changed.set(true);
    win2.w_pos_changed.set(true);

    win_enter(win1, false);
}

fn get_maximum_wincount(fr: &Frame, mut height: i32) -> i32 {
    if fr.fr_layout.get() != FR_COL {
        return height
            / (opt_vars().p_wmh.get() as i32
                + STATUS_HEIGHT
                + frame2win(fr).w_winbar_height.get());
    } else if global_winbar_height() != 0 {
        return height / (opt_vars().p_wmh.get() as i32 + STATUS_HEIGHT + 1);
    }

    let mut total_wincount: i32 = 0;
    for frp in frame_iter_ptr(fr.fr_child.get()) {
        let wp = frame2win(&frp);
        if OptInt::from(height)
            < (opt_vars().p_wmh.get() + OptInt::from(STATUS_HEIGHT + wp.w_winbar_height.get()))
        {
            break;
        }
        height -= opt_vars().p_wmh.get() as i32 + STATUS_HEIGHT + wp.w_winbar_height.get();
        total_wincount += 1;
    }

    total_wincount += height / (opt_vars().p_wmh.get() as i32 + STATUS_HEIGHT);
    total_wincount
}

pub fn win_equal(next_curwin: Option<&WinInfoInfo>, current: bool, mut dir: i32) {
    if dir == 0 {
        dir = i32::from(opt_vars().p_ead.first_byte());
    }
    win_equal_rec(
        match next_curwin {
            Some(w) => crate::globals::WinPtr::from(w),
            None => curwin_ptr(),
        },
        current,
        &topframe_mut(),
        dir,
        0,
        tabline_height(),
        g_st().columns_global.get(),
        crate::globals::topframe_mut().fr_height.get(),
    );
    if !next_curwin.is_some_and(is_ctx_win) {
        win_fix_scroll(true);
    }
}

fn win_equal_rec(
    next_curwin: crate::globals::WinPtr,
    current: bool,
    topfr: &Frame,
    dir: i32,
    mut col: i32,
    mut row: i32,
    mut width: i32,
    mut height: i32,
) {
    let mut extra_sep: i32 = 0;
    let mut totwincount: i32 = 0;
    let mut next_curwin_size: i32 = 0;
    let mut room: i32 = 0;
    let mut has_next_curwin = false;

    if topfr.fr_layout.get() == FR_LEAF {
        if topfr.fr_height.get() != height
            || topfr.win_mut().w_winrow.get() != row
            || topfr.fr_width.get() != width
            || topfr.win_mut().w_wincol.get() != col
        {
            topfr.win_mut().w_winrow.set(row);
            frame_new_height(topfr, height, false, false, false);
            topfr.win_mut().w_wincol.set(col);
            frame_new_width(topfr, width, false, false);
            redraw_all_later(UPD_NOT_VALID);
        }
    } else if topfr.fr_layout.get() == FR_ROW {
        topfr.fr_width.set(width);
        topfr.fr_height.set(height);

        if dir != b'v' as i32 {
            let mut n = frame_minwidth(topfr, nowin());
            if col + width == g_st().columns_global.get() {
                extra_sep = 1;
            } else {
                extra_sep = 0;
            }
            totwincount = (n + extra_sep) / (opt_vars().p_wmw.get() as i32 + 1);
            has_next_curwin = frame_has_win(topfr, next_curwin.opt());

            let m = frame_minwidth(topfr, next_curwin.clone().clone());
            room = width - m;
            if room < 0 {
                next_curwin_size = opt_vars().p_wiw.get() as i32 + room;
                room = 0;
            } else {
                next_curwin_size = -1;
                for fr in frame_iter_ptr(topfr.fr_child.get()) {
                    if frame_fixed_width(&fr) {
                        n = frame_minwidth(&fr, nowin());
                        let mut new_size = fr.fr_width.get();
                        if frame_has_win(&fr, next_curwin.opt()) {
                            room += opt_vars().p_wiw.get() as i32 - opt_vars().p_wmw.get() as i32;
                            next_curwin_size = 0;
                            new_size = new_size.max(opt_vars().p_wiw.get() as i32);
                        } else {
                            totwincount -= (n + if fr.fr_next.get().is_null() {
                                extra_sep
                            } else {
                                0
                            }) / (opt_vars().p_wmw.get() as i32 + 1);
                        }
                        room -= new_size - n;
                        if room < 0 {
                            new_size += room;
                            room = 0;
                        }
                        fr.fr_newwidth.set(new_size);
                    }
                }
                if next_curwin_size == -1 {
                    if !has_next_curwin {
                        next_curwin_size = 0;
                    } else if totwincount > 1
                        && OptInt::from((room + (totwincount - 2)) / (totwincount - 1))
                            > opt_vars().p_wiw.get()
                    {
                        next_curwin_size = ((OptInt::from(room)
                            + opt_vars().p_wiw.get()
                            + OptInt::from(totwincount - 1) * opt_vars().p_wmw.get()
                            + OptInt::from(totwincount - 1))
                            as i32)
                            / totwincount;
                        room -= next_curwin_size - opt_vars().p_wiw.get() as i32;
                    } else {
                        next_curwin_size = opt_vars().p_wiw.get() as i32;
                    }
                }
            }

            if has_next_curwin {
                totwincount -= 1;
            }
        }

        for fr in frame_iter_ptr(topfr.fr_child.get()) {
            let mut wincount: i32 = 1;
            let new_size: i32;
            if fr.fr_next.get().is_null() {
                new_size = width;
            } else if dir == b'v' as i32 {
                new_size = fr.fr_width.get();
            } else if frame_fixed_width(&fr) {
                new_size = fr.fr_newwidth.get();
                wincount = 0;
            } else {
                let n = frame_minwidth(&fr, nowin());
                wincount = (n + if fr.fr_next.get().is_null() {
                    extra_sep
                } else {
                    0
                }) / (opt_vars().p_wmw.get() as i32 + 1);
                let m = frame_minwidth(&fr, next_curwin.clone().clone());
                let hnc = has_next_curwin && frame_has_win(&fr, next_curwin.opt());
                if hnc {
                    wincount -= 1;
                }
                let mut ns = if totwincount == 0 {
                    room
                } else {
                    (wincount * room + (totwincount / 2)) / totwincount
                };
                if hnc {
                    next_curwin_size -= opt_vars().p_wiw.get() as i32 - (m - n);
                    next_curwin_size = next_curwin_size.max(0);
                    ns += next_curwin_size;
                    room -= ns - next_curwin_size;
                } else {
                    room -= ns;
                }
                new_size = ns + n;
            }

            if !current
                || dir != b'v' as i32
                || !topfr.fr_parent.get().is_null()
                || new_size != fr.fr_width.get()
                || frame_has_win(&fr, next_curwin.opt())
            {
                win_equal_rec(
                    next_curwin.clone(),
                    current,
                    &fr,
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
        topfr.fr_width.set(width);
        topfr.fr_height.set(height);

        if dir != b'h' as i32 {
            let mut n = frame_minheight(topfr, nowin());
            if row + height >= g_st().cmdline_row.get() && opt_vars().p_ls.get() == 0 {
                extra_sep = STATUS_HEIGHT;
            } else if global_stl_height() > 0 {
                extra_sep = 1;
            } else {
                extra_sep = 0;
            }
            totwincount = get_maximum_wincount(topfr, n + extra_sep);
            has_next_curwin = frame_has_win(topfr, next_curwin.opt());

            let m = frame_minheight(topfr, next_curwin.clone().clone());
            room = height - m;
            if room < 0 {
                next_curwin_size = opt_vars().p_wh.get() as i32 + room;
                room = 0;
            } else {
                next_curwin_size = -1;
                for fr in frame_iter_ptr(topfr.fr_child.get()) {
                    if frame_fixed_height(&fr) {
                        n = frame_minheight(&fr, nowin());
                        let mut new_size = fr.fr_height.get();
                        if frame_has_win(&fr, next_curwin.opt()) {
                            room += opt_vars().p_wh.get() as i32 - opt_vars().p_wmh.get() as i32;
                            next_curwin_size = 0;
                            new_size = new_size.max(opt_vars().p_wh.get() as i32);
                        } else {
                            totwincount -= get_maximum_wincount(
                                &fr,
                                n + if fr.fr_next.get().is_null() {
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
                        fr.fr_newheight.set(new_size);
                    }
                }
                if next_curwin_size == -1 {
                    if !has_next_curwin {
                        next_curwin_size = 0;
                    } else if totwincount > 1
                        && OptInt::from((room + (totwincount - 2)) / (totwincount - 1))
                            > opt_vars().p_wh.get()
                    {
                        next_curwin_size = ((OptInt::from(room)
                            + opt_vars().p_wh.get()
                            + OptInt::from(totwincount - 1) * opt_vars().p_wmh.get()
                            + OptInt::from(totwincount - 1))
                            as i32)
                            / totwincount;
                        room -= next_curwin_size - opt_vars().p_wh.get() as i32;
                    } else {
                        next_curwin_size = opt_vars().p_wh.get() as i32;
                    }
                }
            }

            if has_next_curwin {
                totwincount -= 1;
            }
        }

        for fr in frame_iter_ptr(topfr.fr_child.get()) {
            let mut wincount: i32 = 1;
            let new_size: i32;
            if fr.fr_next.get().is_null() {
                new_size = height;
            } else if dir == b'h' as i32 {
                new_size = fr.fr_height.get();
            } else if frame_fixed_height(&fr) {
                new_size = fr.fr_newheight.get();
                wincount = 0;
            } else {
                let n = frame_minheight(&fr, nowin());
                wincount = get_maximum_wincount(
                    &fr,
                    n + if fr.fr_next.get().is_null() {
                        extra_sep
                    } else {
                        0
                    },
                );
                let m = frame_minheight(&fr, next_curwin.clone().clone());
                let hnc = has_next_curwin && frame_has_win(&fr, next_curwin.opt());
                if hnc {
                    wincount -= 1;
                }
                let mut ns = if totwincount == 0 {
                    room
                } else {
                    (wincount * room + (totwincount / 2)) / totwincount
                };
                if hnc {
                    next_curwin_size -= opt_vars().p_wh.get() as i32 - (m - n);
                    ns += next_curwin_size;
                    room -= ns - next_curwin_size;
                } else {
                    room -= ns;
                }
                new_size = ns + n;
            }
            if !current
                || dir != b'h' as i32
                || !topfr.fr_parent.get().is_null()
                || new_size != fr.fr_height.get()
                || frame_has_win(&fr, next_curwin.opt())
            {
                win_equal_rec(
                    next_curwin.clone(),
                    current,
                    &fr,
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

pub fn leaving_window(win: &WinInfoInfo) {
    if !bt_prompt(win.buf_opt().as_deref()) || is_ctx_win(win) {
        return;
    }

    win.buf_mut().b_prompt_insert.set(g_st().restart_edit.get());
    if g_st().restart_edit.get() != NUL as i32 && g_st().mode_displayed.get() {
        g_st().clear_cmdline.set(true);
    }
    g_st().restart_edit.set(NUL as i32);

    if (g_st().state_global.get() & MODE_INSERT) != 0
        && !g_st().ins_global.borrow().stop_insert_mode
    {
        g_st().ins_global.borrow_mut().stop_insert_mode = true;
        if win.buf_mut().b_prompt_insert.get() == NUL as i32 {
            win.buf_mut().b_prompt_insert.set(b'A' as i32);
        }
    }
}

pub fn entering_window(win: &WinInfoInfo) {
    if !bt_prompt(win.buf_opt().as_deref()) || is_ctx_win(win) {
        return;
    }

    if win.buf_mut().b_prompt_insert.get() != NUL as i32 {
        g_st().ins_global.borrow_mut().stop_insert_mode = false;
    }

    if (g_st().state_global.get() & MODE_INSERT) == 0 {
        g_st().restart_edit.set(win.buf_mut().b_prompt_insert.get());
    }
}

pub fn win_init_empty(wp: &WinInfoInfo) {
    redraw_later(Some(wp), UPD_NOT_VALID);
    wp.w_lines_valid.set(0);
    wp.w_cursor.set_lnum(1);
    wp.w_cursor.set_col(0);
    wp.w_curswant.set(0);
    wp.w_cursor.set_coladd(0);
    wp.w_pcmark.set_lnum(1);
    wp.w_pcmark.set_col(0);
    wp.w_prev_pcmark.set_lnum(0);
    wp.w_prev_pcmark.set_col(0);
    wp.w_topline.set(1);
    wp.w_topfill.set(0);
    wp.w_botline.set(2);
    wp.w_valid.set(0);
    wp.w_s_owned.set(None);
}

pub fn curwin_init() {
    win_init_empty(&curwin_mut());
}

pub fn close_windows(buf_in: Option<&FileBuf>, keep_curwin: bool) {
    let buf = crate::globals::BufPtr::from(buf_in);
    g_st()
        .redrawing_disabled
        .set(g_st().redrawing_disabled.get() + (1));

    let mut wp = g_st().lastwin.get();
    while !wp.is_null() && (is_ctx_win(&lastwin_mut()) || !one_window(wp.borrow_mut(), None)) {
        if wp.borrow().w_buffer.get() == buf
            && (!keep_curwin || wp != curwin_handle())
            && !(win_locked(wp.borrow_mut()) != 0 || wp.borrow().buf_mut().b_locked.get() > 0)
        {
            if window_layout_locked(Cmdidx::CmdSize) {
                g_st()
                    .redrawing_disabled
                    .set(g_st().redrawing_disabled.get() - (1));
                return;
            }
            if !win_close(wp.borrow_mut(), false, false) {
                break;
            }
            wp = g_st().lastwin.get();
        } else {
            wp = wp.borrow().w_prev.get();
        }
    }

    let mut tp = g_st().first_tabpage.get().clone();
    while !tp.is_null() {
        let mut nexttp = tp.tp_next.get();
        if tp != g_st().curtab.get() {
            let mut wp2 = tp.tp_lastwin.get();
            while !wp2.is_null() {
                if wp2.borrow().w_buffer.get() == buf
                    && !(win_locked(wp2.borrow_mut()) != 0
                        || wp2.borrow().buf_mut().b_locked.get() > 0)
                {
                    if window_layout_locked(Cmdidx::CmdSize) {
                        g_st()
                            .redrawing_disabled
                            .set(g_st().redrawing_disabled.get() - (1));
                        return;
                    }
                    if !win_close_othertab(wp2.borrow_mut(), 0, tp.borrow(), false) {
                        g_st()
                            .redrawing_disabled
                            .set(g_st().redrawing_disabled.get() - (1));
                        return;
                    }
                    nexttp = g_st().first_tabpage.get().clone();
                    break;
                }
                wp2 = wp2.borrow().w_prev.get();
            }
        }
        tp = nexttp;
    }

    g_st()
        .redrawing_disabled
        .set(g_st().redrawing_disabled.get() - (1));
}

pub fn last_window(win: &WinInfoInfo) -> bool {
    one_window(win, None) && crate::globals::first_tabpage_mut().tp_next.get().is_null()
}

pub fn one_window(win: &WinInfoInfo, tp: Option<&TabPageInfo>) -> bool {
    let first = tp.map_or_else(|| g_st().firstwin.get(), |t| t.tp_firstwin.get());
    debug_assert!(tp.is_none_or(|t| g_st().curtab.get() != t) && !first.w_floating.get());
    first == win && win.next_opt().is_none_or(|next| next.w_floating.get())
}

fn can_close_floating_windows(tp: Option<&TabPageInfo>) -> bool {
    debug_assert!(
        tp.is_none_or(|t| g_st().curtab.get() != t)
            && (tp.is_some() || !is_ctx_win(&lastwin_mut()))
    );
    let mut wp = tp.map_or_else(|| g_st().lastwin.get(), |t| t.tp_lastwin.get());
    while wp.w_floating.get() {
        let buf = wp.w_buffer.get();
        let need_hide = buf_is_changed_fn(&buf) && buf.b_nwindows.get() <= 1;
        if need_hide && !buf_hide(&buf) {
            return false;
        }

        wp = wp.w_prev.get();
    }
    true
}

fn close_last_window_tabpage(
    win_in: &WinInfoInfo,
    mut free_buf: bool,
    prev_curtab_in: &TabPageInfo,
) -> bool {
    let win = crate::globals::WinPtr::from(win_in);
    let prev_curtab = crate::globals::TabpagePtr::from(prev_curtab_in);
    if !one_window_macro() {
        return false;
    }

    let old_curbuf = g_st().curbuf.get();
    let save_lastused = g_st().lastused_tabpage.get().clone();

    let term = if !win.borrow_mut().w_buffer.get().is_null() {
        win.borrow_mut().buf_mut().terminal.get()
    } else {
        crate::terminal::TermId::NULL
    };
    if !term.is_null() {
        free_buf = false;
    }

    goto_tabpage_tp(
        alt_tabpage().as_deref(),
        false,
        !win.borrow_mut().w_buffer.get().is_null(),
    );

    if g_st().curtab.get() != prev_curtab
        && valid_tabpage(prev_curtab.opt())
        && prev_curtab.tp_firstwin.get() == win
    {
        win_close_othertab(
            win.borrow_mut(),
            i32::from(free_buf),
            prev_curtab.borrow(),
            false,
        );
    }

    if valid_tabpage(save_lastused.opt()) && g_st().curtab.get() != save_lastused {
        g_st().lastused_tabpage.set(save_lastused);
    }
    entering_window(&curwin_mut());

    apply_autocmds(
        EVENT_WINENTER,
        None,
        None,
        false,
        curbuf_opt_mut().as_deref(),
    );
    apply_autocmds(
        EVENT_TABENTER,
        None,
        None,
        false,
        curbuf_opt_mut().as_deref(),
    );
    if g_st().curbuf.get() != old_curbuf {
        apply_autocmds(
            EVENT_BUFENTER,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
    }
    true
}

fn win_close_buffer(win_in: &WinInfoInfo, action: i32, abort_if_last: bool) -> bool {
    let win = crate::globals::WinPtr::from(win_in);
    if !win.borrow_mut().w_buffer.get().is_null() {
        reset_synblock(win.borrow_mut());
    }

    if !win.borrow_mut().w_buffer.get().is_null()
        && bt_quickfix(win.w_buffer.get().opt())
        && win.borrow_mut().buf_mut().b_nwindows.get() == 1
    {
        win.borrow_mut().buf_mut().b_p_bl.set(false);
    }

    let mut retval = false;
    if !win.borrow_mut().w_buffer.get().is_null() {
        let mut bufref: Bufref = Bufref::new();
        set_bufref(&mut bufref, g_st().curbuf.get().opt_mut());

        win.borrow_mut()
            .w_locked
            .set(win.borrow_mut().w_locked.get() + (1));

        retval = close_buffer(
            win.opt_mut(),
            win.w_buffer.get().opt_mut(),
            action,
            abort_if_last,
            true,
            true,
        );

        if win_valid_any_tab(win.opt()) {
            win.borrow_mut()
                .w_locked
                .set(win.borrow_mut().w_locked.get() - (1));
        }

        if !bufref_valid(Some(&bufref)) {
            g_st().curbuf.set(g_st().firstbuf.get());
        }
    }

    retval
}

fn win_unclose_buffer(win: &WinInfoInfo, _bufref: &mut Bufref) {
    if win.w_buffer.get().is_null() {
        win.w_buffer.set(g_st().firstbuf.get());
        win.buf_mut()
            .b_nwindows
            .set(win.buf_mut().b_nwindows.get() + (1));
        if win == curwin_ptr() {
            g_st().curbuf.set(curwin_mut().w_buffer.get());
        }
        win_init_empty(win);
    }
}

pub fn win_close(win_in: &WinInfoInfo, free_buf: bool, force: bool) -> bool {
    let win = crate::globals::WinPtr::from(win_in);
    let prev_curtab = g_st().curtab.get().clone();
    let win_frame = if win.borrow_mut().w_floating.get() {
        crate::globals::FramePtr::NULL
    } else {
        win.borrow_mut().frame_mut().fr_parent.get()
    };
    let had_diffmode = win.borrow_mut().w_onebuf_opt.wo_diff.get();

    if last_window(win.borrow_mut()) {
        emsg(gettext(E_CANNOT_CLOSE_LAST_WINDOW));
        return false;
    }
    if !win.borrow_mut().w_floating.get() && window_layout_locked(Cmdidx::CmdClose) {
        return false;
    }

    if win_locked(win.borrow_mut()) != 0
        || !win.borrow_mut().w_buffer.get().is_null()
            && win.borrow_mut().buf_mut().b_locked.get() > 0
    {
        return false;
    }
    if is_ctx_win(win.borrow()) {
        emsg(gettext(E_AUTOCMD_CLOSE));
        return false;
    }
    if lastwin_mut().w_floating.get() && one_window(win.borrow_mut(), None) {
        if is_ctx_win(&lastwin_mut()) {
            emsg(gettext(
                c"E814: Cannot close window, only autocmd window would remain",
            ));
            return false;
        }
        if force || can_close_floating_windows(None) {
            while lastwin_mut().w_floating.get() {
                let hide = !buf_hide(&lastwin_mut().buf_mut());
                if !win_close(&lastwin_mut(), hide, true) {
                    return false;
                }
            }
            if !win_valid_any_tab(win.opt()) {
                return false;
            }
            if last_window(win.borrow_mut()) {
                emsg(gettext(E_CANNOT_CLOSE_LAST_WINDOW));
                return false;
            }
        } else {
            emsg(E_FLOATONLY);
            return false;
        }
    }

    if close_last_window_tabpage(win.borrow_mut(), free_buf, prev_curtab.borrow()) {
        return false;
    }

    let mut help_window = false;
    let mut quickfix_window = false;

    if bt_help(win.w_buffer.get().opt()) {
        help_window = true;
    } else {
        clear_snapshot(&curtab_mut(), SNAP_HELP_IDX as i32);
    }

    if bt_quickfix(win.w_buffer.get().opt()) {
        quickfix_window = true;
    } else {
        clear_snapshot(&curtab_mut(), SNAP_QUICKFIX_IDX as i32);
    }

    let mut other_buffer = false;

    if win == curwin_ptr() {
        leaving_window(&curwin_mut());

        let wp = if win.borrow_mut().w_floating.get() {
            win_float_find_altwin(win.borrow(), None)
        } else {
            frame2win(&win_altframe(win.borrow_mut(), None))
        };

        if wp.w_buffer.get() != g_st().curbuf.get() {
            reset_visual_and_resel();

            other_buffer = true;
            if !win_valid(win.opt()) {
                return false;
            }

            win.borrow_mut()
                .w_locked
                .set(win.borrow_mut().w_locked.get() + (1));

            apply_autocmds(
                EVENT_BUFLEAVE,
                None,
                None,
                false,
                curbuf_opt_mut().as_deref(),
            );
            if !win_valid(win.opt()) {
                return false;
            }

            win.borrow_mut()
                .w_locked
                .set(win.borrow_mut().w_locked.get() - (1));

            if last_window(win.borrow_mut()) {
                return false;
            }
        }

        win.borrow_mut()
            .w_locked
            .set(win.borrow_mut().w_locked.get() + (1));

        apply_autocmds(
            EVENT_WINLEAVE,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
        if !win_valid(win.opt()) {
            return false;
        }

        win.borrow_mut()
            .w_locked
            .set(win.borrow_mut().w_locked.get() - (1));

        if last_window(win.borrow_mut()) {
            return false;
        }
        if aborting() {
            return false;
        }
    }

    do_autocmd_winclosed(win.borrow_mut());

    if !win_valid_any_tab(win.opt()) {
        return true;
    }

    let mut bufref: Bufref = Bufref::new();

    set_bufref(&mut bufref, win.w_buffer.get().opt_mut());

    win_close_buffer(
        win.borrow_mut(),
        if free_buf { DOBUF_UNLOAD } else { 0 },
        true,
    );

    if win_valid(win.opt())
        && win.borrow_mut().w_buffer.get().is_null()
        && !win.borrow_mut().w_floating.get()
        && last_window(win.borrow_mut())
    {
        if curwin_mut().w_buffer.get().is_null() {
            curwin_mut().w_buffer.set(g_st().curbuf.get());
        }
        getout(0);
    }
    if g_st().curtab.get() != prev_curtab
        && win_valid_any_tab(win.opt())
        && win.borrow_mut().w_buffer.get().is_null()
    {
        win_close_othertab(win.borrow_mut(), 0, prev_curtab.borrow(), force);
        return false;
    }

    if !win_valid(win.opt()) {
        return false;
    }
    if one_window(win.borrow_mut(), None)
        && (crate::globals::first_tabpage_mut().tp_next.get().is_null()
            || lastwin_mut().w_floating.get())
    {
        if !crate::globals::first_tabpage_mut().tp_next.get().is_null() {
            emsg(E_FLOATONLY);
        }

        win_unclose_buffer(win.borrow_mut(), &mut bufref);

        return false;
    }
    if close_last_window_tabpage(win.borrow_mut(), free_buf, prev_curtab.borrow()) {
        return false;
    }

    WIN_STATE.with(|s| s.split_disallowed.set(s.split_disallowed.get() + (1)));

    let was_floating = win.borrow_mut().w_floating.get();
    if ui_has(K_UIMULTIGRID) {
        ui_call_win_close(Integer::from(win.borrow_mut().w_grid_alloc.handle.get()));
    }

    if win.borrow_mut().w_floating.get() {
        ui_comp_remove_grid(&win.borrow_mut().w_grid_alloc);

        debug_assert!(!g_st().first_tabpage.get().is_null());
        if win.borrow_mut().w_config.borrow().external {
            for tp in for_all_tabpages_ptr() {
                if g_st().curtab.get() != tp && tp.tp_curwin.get() == win {
                    tp.tp_curwin.set(tp.tp_firstwin.get());
                }
            }
        }
    }

    set_bufref(&mut bufref, win.w_buffer.get().opt_mut());

    if !win.borrow_mut().w_buffer.get().is_null() {
        win.borrow_mut()
            .buf_mut()
            .b_nwindows
            .set(win.borrow_mut().buf_mut().b_nwindows.get() - (1));
    }

    let had_cmdline_ruler =
        opt_vars().p_ru.get() && win == curwin_ptr() && win.borrow_mut().w_status_height.get() == 0;
    let mut dir: i32 = 0;
    let mut wp = crate::globals::WinPtr::from(win_free_mem(win.borrow_mut(), &mut dir, None));

    if help_window || quickfix_window {
        let prev_win = crate::globals::WinPtr::from(get_snapshot_curwin(if help_window {
            SNAP_HELP_IDX as i32
        } else {
            SNAP_QUICKFIX_IDX as i32
        }));
        if win_valid(prev_win.opt()) {
            wp = prev_win;
        }
    }

    let mut close_curwin = false;

    if win == curwin_ptr() {
        set_curwin(&wp);
        if (wp.borrow_mut()).w_onebuf_opt.wo_pvw.get() || bt_quickfix(wp.w_buffer.get().opt()) {
            loop {
                if (wp.borrow_mut()).w_next.get().is_null() {
                    wp = g_st().firstwin.get();
                } else {
                    wp = wp.w_next.get();
                }
                if wp == curwin_ptr() {
                    break;
                }
                if !(wp.borrow_mut()).w_onebuf_opt.wo_pvw.get()
                    && !bt_quickfix(wp.w_buffer.get().opt())
                    && !((wp.borrow_mut()).w_floating.get() && !win_is_focusable(wp.borrow()))
                {
                    set_curwin(&wp);
                    break;
                }
            }
        }
        g_st().curbuf.set(curwin_mut().w_buffer.get());
        close_curwin = true;

        check_cursor(&curwin_mut());
    }

    if !was_floating {
        last_status(false);

        if !curwin_mut().w_floating.get()
            && opt_vars().p_ea.get()
            && (opt_vars().p_ead.first_byte() == b'b'
                || i32::from(opt_vars().p_ead.first_byte()) == dir)
        {
            win_equal(
                curwin_ptr().opt(),
                curwin_mut().frame_mut().fr_parent.get() == win_frame,
                dir,
            );
        } else {
            win_comp_pos();
            win_fix_scroll(false);
        }
    } else if had_cmdline_ruler && (wp.borrow_mut()).w_status_height.get() > 0 {
        g_st().redraw_cmdline.set(true);
    }
    if !bufref.br_buf.is_null()
        && bufref_valid(Some(&bufref))
        && !bufref.br_buf.terminal.get().is_null()
    {
        terminal_check_size(bufref.br_buf.terminal.get().borrow_mut());
    }

    if close_curwin {
        win_enter_ext(
            wp.borrow_mut(),
            WEE_CURWIN_INVALID | WEE_TRIGGER_ENTER_AUTOCMDS | WEE_TRIGGER_LEAVE_AUTOCMDS,
        );
        if other_buffer {
            apply_autocmds(
                EVENT_BUFENTER,
                None,
                None,
                false,
                curbuf_opt_mut().as_deref(),
            );
        }
    }

    if one_window_macro()
        && curwin_mut().w_locked.get() != 0
        && curbuf_mut().b_locked_split.get() != 0
        && !crate::globals::first_tabpage_mut().tp_next.get().is_null()
    {
        apply_autocmds(
            EVENT_TABLEAVE,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
    }

    WIN_STATE.with(|s| s.split_disallowed.set(s.split_disallowed.get() - (1)));

    if help_window || quickfix_window {
        restore_snapshot(
            if help_window {
                SNAP_HELP_IDX as i32
            } else {
                SNAP_QUICKFIX_IDX as i32
            },
            i32::from(close_curwin),
        );
    }

    if diffopt_closeoff() && had_diffmode && g_st().curtab.get() == prev_curtab {
        let mut diffcount: i32 = 0;
        for dwin in for_all_windows_mut() {
            if dwin.w_onebuf_opt.wo_diff.get() {
                diffcount += 1;
            }
        }
        if diffcount == 1 {
            do_cmdline_cmd(c"diffoff!");
        }
    }

    curwin_mut().w_pos_changed.set(true);
    if !was_floating {
        redraw_all_later(UPD_NOT_VALID);
    }
    true
}

fn trigger_winnewpre() {
    window_layout_lock();
    apply_autocmds(EVENT_WINNEWPRE, None, None, false, None);
    window_layout_unlock();
}

fn do_autocmd_winclosed(win: &WinInfoInfo) {
    if WIN_STATE.with(|s| s.winclosed_recursive.get()) || !has_event(EVENT_WINCLOSED) {
        return;
    }
    WIN_STATE.with(|s| s.winclosed_recursive.set(true));
    let mut winid = [0u8; NUMBUFLEN];
    format_to_buf!(&mut winid[..], c"%d", win.handle);
    let winid_cstr = crate::memory::bytes_as_cstr(&winid);
    apply_autocmds_win(
        EVENT_WINCLOSED,
        Some(winid_cstr),
        Some(winid_cstr),
        false,
        win.w_buffer.get().opt_mut(),
        Some(win),
    );
    WIN_STATE.with(|s| s.winclosed_recursive.set(false));
}

pub fn trigger_tabclosedpre(tp_in: &TabPageInfo) {
    let tp = crate::globals::TabpagePtr::from(tp_in);
    let ptp = g_st().curtab.get().clone();

    if !has_event(EVENT_TABCLOSEDPRE) || WIN_STATE.with(|s| s.tabclosedpre_recursive.get()) {
        return;
    }

    if valid_tabpage(tp.opt()) {
        goto_tabpage_tp(tp.opt(), false, false);
    }
    WIN_STATE.with(|s| s.tabclosedpre_recursive.set(true));
    window_layout_lock();
    apply_autocmds(EVENT_TABCLOSEDPRE, None, None, false, None);
    window_layout_unlock();
    WIN_STATE.with(|s| s.tabclosedpre_recursive.set(false));
    if valid_tabpage(ptp.opt()) {
        goto_tabpage_tp(ptp.opt(), false, false);
    } else {
        goto_tabpage_tp(g_st().first_tabpage.get().clone().opt(), false, false);
    }
}

pub fn win_close_othertab(
    win_in: &WinInfoInfo,
    free_buf: i32,
    tp_in: &TabPageInfo,
    force: bool,
) -> bool {
    let win = crate::globals::WinPtr::from(win_in);
    let tp = crate::globals::TabpagePtr::from(tp_in);
    debug_assert!(g_st().curtab.get() != tp);

    if window_layout_locked(Cmdidx::CmdSize) {
        return false;
    }
    if win_locked(win.borrow_mut()) != 0
        || !win.borrow_mut().w_buffer.get().is_null()
            && win.borrow_mut().buf_mut().b_locked.get() > 0
    {
        return false;
    }
    if is_ctx_win(win.borrow()) {
        emsg(gettext(E_AUTOCMD_CLOSE));
        return false;
    }

    if tp.tp_lastwin.get().borrow().w_floating.get() && one_window(win.borrow_mut(), tp.opt()) {
        if force || can_close_floating_windows(tp.opt()) {
            loop {
                if !tp.tp_lastwin.get().borrow().w_floating.get() {
                    break;
                }
                let last_w = tp.tp_lastwin.get();
                let free_b = i32::from(!buf_hide(&last_w.w_buffer.get()));
                if !win_close_othertab(&last_w, free_b, tp.borrow(), true) {
                    return false;
                }
            }
            if !win_valid_any_tab(win.opt()) {
                return false;
            }
        } else {
            emsg(E_FLOATONLY);
            return false;
        }
    }

    if !win.borrow_mut().w_buffer.get().is_null() {
        do_autocmd_winclosed(win.borrow_mut());

        if !win_valid_any_tab(win.opt()) {
            return false;
        }
    }

    if ptr::eq(tp.tp_firstwin.get().as_ptr(), tp.tp_lastwin.get().as_ptr())
        && !tp.tp_did_tabclosedpre.get()
    {
        trigger_tabclosedpre(tp.borrow());

        if !win_valid_any_tab(win.opt()) {
            return false;
        }
    }

    let mut bufref: Bufref = Bufref::new();

    set_bufref(&mut bufref, win.w_buffer.get().opt_mut());

    if !win.borrow_mut().w_buffer.get().is_null() {
        close_buffer(
            win.opt_mut(),
            win.w_buffer.get().opt_mut(),
            if free_buf != 0 { DOBUF_UNLOAD } else { 0 },
            false,
            true,
            true,
        );
    }

    if !valid_tabpage(tp.opt())
        || g_st().curtab.get() == tp
        || !tabpage_win_valid(tp.opt(), win.opt())
    {
        if win_valid_any_tab(win.opt()) {
            win_unclose_buffer(win.borrow_mut(), &mut bufref);
        }
        return false;
    }
    if tp.tp_lastwin.get().borrow().w_floating.get() && one_window(win.borrow_mut(), tp.opt()) {
        emsg(E_FLOATONLY);
        if win_valid_any_tab(win.opt()) {
            win_unclose_buffer(win.borrow_mut(), &mut bufref);
        }
        return false;
    }

    let mut free_tp_idx: i32 = 0;

    if ptr::eq(tp.tp_firstwin.get().as_ptr(), tp.tp_lastwin.get().as_ptr()) {
        free_tp_idx = tabpage_index(tp.opt());
        let h = tabline_height();

        if g_st().first_tabpage.get() == tp {
            g_st().first_tabpage.set(tp.tp_next.get());
        } else {
            let mut ptp = g_st().first_tabpage.get().clone();
            while !ptp.is_null() && ptp.tp_next.get() != tp {
                ptp = ptp.tp_next.get();
            }
            if ptp.is_null() {
                internal_error(c"win_close_othertab()");
                return false;
            }

            ptp.tp_next.set(tp.tp_next.get());
        }
        g_st().redraw_tabline.set(true);
        if h != tabline_height() {
            win_new_screen_rows();
        }
    }

    if ui_has(K_UIMULTIGRID) {
        ui_call_win_close(Integer::from(win.borrow_mut().w_grid_alloc.handle.get()));
    }
    if win.borrow_mut().w_floating.get() {
        ui_comp_remove_grid(&win.borrow_mut().w_grid_alloc);
    }

    set_bufref(&mut bufref, win.w_buffer.get().opt_mut());

    if !win.borrow_mut().w_buffer.get().is_null() {
        win.borrow_mut()
            .buf_mut()
            .b_nwindows
            .set(win.borrow_mut().buf_mut().b_nwindows.get() - (1));
    }

    let mut dir: i32 = 0;

    win_free_mem(win.borrow_mut(), &mut dir, tp.opt());

    if !bufref.br_buf.is_null()
        && bufref_valid(Some(&bufref))
        && !bufref.br_buf.terminal.get().is_null()
    {
        terminal_check_size(bufref.br_buf.terminal.get().borrow_mut());
    }
    if free_tp_idx > 0 {
        free_tabpage(tp.opt());

        if has_event(EVENT_TABCLOSED) {
            let mut prev_idx = [0u8; NUMBUFLEN];
            format_to_buf!(&mut prev_idx[..], c"%i", free_tp_idx);
            let prev_idx_cstr = crate::memory::bytes_as_cstr(&prev_idx);
            let curbuf_fallback = curbuf_opt_mut();
            let buf_opt = if !bufref.br_buf.is_null() && bufref_valid(Some(&bufref)) {
                bufref.br_buf.opt_mut()
            } else {
                curbuf_fallback.as_deref()
            };
            apply_autocmds(
                EVENT_TABCLOSED,
                Some(prev_idx_cstr),
                Some(prev_idx_cstr),
                false,
                buf_opt,
            );
        }
    }
    true
}

fn win_free_mem(
    win_in: &WinInfoInfo,
    dirp: &mut i32,
    tp_in: Option<&TabPageInfo>,
) -> Option<crate::globals::WinPtr> {
    let win = crate::globals::WinPtr::from(win_in);
    let tp = crate::globals::TabpagePtr::from(tp_in);
    let wp: crate::globals::WinPtr;
    let win_tp = if tp.is_null() {
        g_st().curtab.get().clone()
    } else {
        tp.clone()
    };

    if !win.borrow_mut().w_floating.get() {
        let frp = win.borrow_mut().w_frame.get().clone();

        wp = crate::globals::WinPtr::from(winframe_remove(
            win.borrow_mut(),
            &mut *dirp,
            tp.opt(),
            None,
        ));

        frp.clear_links();
    } else {
        *dirp = b'h' as i32;

        wp = win_float_find_altwin(win.borrow(), tp.opt());
    }
    win_free(win.borrow_mut(), tp.opt());

    if win_tp.tp_curwin.get() == win {
        win_tp.tp_curwin.set(wp.clone());
    }
    if g_st().cmdline_win.get() == win {
        g_st().cmdline_win.set(crate::globals::WinPtr::NULL);
    }

    (!wp.is_null()).then_some(wp)
}

pub fn win_free_all() {
    g_st().cmdwin_type.set(0);
    g_st().cmdwin_buf.set(crate::globals::BufPtr::NULL);

    while !crate::globals::first_tabpage_mut().tp_next.get().is_null() {
        tabpage_close(true);
    }

    while !g_st().lastwin.get().is_null() && lastwin_mut().w_floating.get() {
        let wp = g_st().lastwin.get();
        win_remove(&lastwin_mut(), None);
        let mut dummy: i32 = 0;
        win_free_mem(wp.borrow_mut(), &mut dummy, None);
        for item in g_st().ctx_win_vec.borrow_mut().iter_mut() {
            if item.cw_win == wp {
                item.cw_win = crate::globals::WinPtr::NULL;
            }
        }
    }

    for i in 0..g_st().ctx_win_vec.borrow().len() {
        let cw = g_st().ctx_win_vec.borrow().as_slice()[i].cw_win.clone();
        if let Some(cw_mut) = cw.opt_mut() {
            let mut dummy: i32 = 0;
            win_free_mem(cw_mut, &mut dummy, None);
            g_st().ctx_win_vec.borrow_mut().as_mut_slice()[i].cw_win = crate::globals::WinPtr::NULL;
        }
    }

    *g_st().ctx_win_vec.borrow_mut() = Vec::new();

    while !g_st().firstwin.get().is_null() {
        let mut dummy: i32 = 0;
        win_free_mem(g_st().firstwin.get().borrow_mut(), &mut dummy, None);
    }

    set_curwin(crate::globals::WinPtr::NULL);
}

pub fn winframe_remove(
    win: &WinInfoInfo,
    dirp: &mut i32,
    tp: Option<&TabPageInfo>,
    unflat_altfr: Option<&mut crate::globals::FramePtr>,
) -> Option<crate::globals::WinPtr> {
    let mut altfr = crate::globals::FramePtr::NULL;
    let wp = winframe_find_altwin(win, dirp, tp, Some(&mut altfr))?;

    let frp_close = win.frame_mut();
    let altfr_ref = altfr.borrow();

    WIN_STATE.with(|s| s.frame_locked.set(s.frame_locked.get() + (1)));

    let topleft = frame2win(&frp_close.parent_mut());
    let mut row = topleft.w_winrow.get();
    let mut col = topleft.w_wincol.get();

    if win.w_vsep_width.get() == 0
        && frp_close.parent_mut().fr_layout.get() == FR_ROW
        && !frp_close.fr_prev.get().is_null()
    {
        frame_set_vsep(&frp_close.prev_mut(), false);
    }

    frame_remove(&frp_close);

    if *dirp == b'v' as i32 {
        let new_h = altfr_ref.fr_height.get() + frp_close.fr_height.get();
        frame_new_height(
            altfr_ref,
            new_h,
            frp_close.fr_next.get() == altfr,
            !frame_fixed_height(altfr.borrow()),
            false,
        );
    } else {
        debug_assert!(*dirp == b'h' as i32);
        let new_w = altfr_ref.fr_width.get() + frp_close.fr_width.get();
        frame_new_width(
            altfr_ref,
            new_w,
            frp_close.fr_next.get() == altfr,
            !frame_fixed_height(altfr.borrow()),
        );
    }

    if frp_close.fr_prev.get() != altfr {
        frame_comp_pos(&frp_close.parent_mut(), &mut row, &mut col);
    }

    if let Some(out) = unflat_altfr {
        *out = altfr;
    } else {
        frame_flatten(altfr.borrow());
    }

    WIN_STATE.with(|s| s.frame_locked.set(s.frame_locked.get() - (1)));

    (!wp.is_null()).then_some(wp)
}

pub fn winframe_find_altwin(
    win: &WinInfoInfo,
    dirp: &mut i32,
    tp: Option<&TabPageInfo>,
    altfr: Option<&mut crate::globals::FramePtr>,
) -> Option<crate::globals::WinPtr> {
    debug_assert!(tp.is_none_or(|t| g_st().curtab.get() != t));

    if one_window(win, tp) {
        return None;
    }

    let frp_close = win.w_frame.get().clone();
    let mut frp2 = win_altframe(win, tp);
    let mut wp = frame2win(frp2.borrow());

    if frp_close.fr_parent.get().fr_layout.get() == FR_COL {
        if !frp2.fr_win.get().is_null() && frp2.fr_win.get().w_onebuf_opt.wo_wfh.get() {
            let mut frp = frp_close.fr_prev.get();
            let mut frp3 = frp_close.fr_next.get();
            while !frp.is_null() || !frp3.is_null() {
                if !frp.is_null() {
                    if !frame_fixed_height(&frp) {
                        frp2 = frp;
                        wp = frame2win(frp2.borrow());
                        break;
                    }

                    frp = frp.fr_prev.get();
                }
                if !frp3.is_null() {
                    if !frp3.fr_win.get().is_null() && !frp3.fr_win.get().w_onebuf_opt.wo_wfh.get()
                    {
                        frp2 = frp3;
                        wp = frp2.fr_win.get();
                        break;
                    }

                    frp3 = frp3.fr_next.get();
                }
            }
        }
        *dirp = b'v' as i32;
    } else {
        if !frp2.fr_win.get().is_null() && frp2.fr_win.get().w_onebuf_opt.wo_wfw.get() {
            let mut frp = frp_close.fr_prev.get();
            let mut frp3 = frp_close.fr_next.get();
            while !frp.is_null() || !frp3.is_null() {
                if !frp.is_null() {
                    if !frame_fixed_width(&frp) {
                        frp2 = frp;
                        wp = frame2win(frp2.borrow());
                        break;
                    }

                    frp = frp.fr_prev.get();
                }
                if !frp3.is_null() {
                    if !frp3.fr_win.get().is_null() && !frp3.fr_win.get().w_onebuf_opt.wo_wfw.get()
                    {
                        frp2 = frp3;
                        wp = frp2.fr_win.get();
                        break;
                    }

                    frp3 = frp3.fr_next.get();
                }
            }
        }
        *dirp = b'h' as i32;
    }

    debug_assert!(wp != win && frp2 != frp_close);
    if let Some(out) = altfr {
        *out = frp2;
    }

    (!wp.is_null()).then_some(wp)
}

fn frame_flatten(frp_in: &Frame) {
    let mut frp = crate::globals::FramePtr::from(frp_in);
    if !frp.fr_next.get().is_null() || !frp.fr_prev.get().is_null() {
        return;
    }

    frp.fr_parent.get().fr_layout.set(frp.fr_layout.get());

    frp.fr_parent.get().fr_child.set(frp.fr_child.get());

    let mut frp2 = frp.fr_child.get();
    while !frp2.is_null() {
        frp2.fr_parent.set(frp.fr_parent.get());

        frp2 = frp2.fr_next.get();
    }

    frp.fr_parent.get().fr_win.set(frp.fr_win.get());

    if !frp.fr_win.get().is_null() {
        frp.fr_win
            .get()
            .borrow_mut()
            .w_frame
            .set(frp.fr_parent.get());
    }

    frp2 = frp.fr_parent.get();

    if crate::globals::topframe_mut().fr_child.get() == frp {
        crate::globals::topframe_mut().fr_child.set(frp2.clone());
    }

    frp.clear_links();

    frp = frp2.fr_parent.get();

    if !frp.is_null() && frp.fr_layout.get() == frp2.fr_layout.get() {
        if frp.fr_child.get() == frp2 {
            frp.fr_child.set(frp2.fr_child.get());
        }

        debug_assert!(!frp2.fr_child.get().is_null());

        frp2.fr_child.get().fr_prev.set(frp2.fr_prev.get());

        if !frp2.fr_prev.get().is_null() {
            frp2.fr_prev.get().fr_next.set(frp2.fr_child.get());
        }
        let mut frp3 = frp2.fr_child.get();
        loop {
            frp3.fr_parent.set(frp.clone());

            if frp3.fr_next.get().is_null() {
                frp3.fr_next.set(frp2.fr_next.get());

                if !frp2.fr_next.get().is_null() {
                    frp2.fr_next.get().fr_prev.set(frp3);
                }
                break;
            }

            frp3 = frp3.fr_next.get();
        }
        if crate::globals::topframe_mut().fr_child.get() == frp2 {
            crate::globals::topframe_mut().fr_child.set(frp);
        }

        frp2.clear_links();
    }
}

pub fn winframe_restore(wp: &WinInfoInfo, dir: i32, unflat_altfr: &Frame) {
    let frp = wp.frame_mut();

    if let Some(prev) = frp.prev_opt() {
        frame_append(&prev, &frp);
    } else {
        frame_insert(&frp.next_mut(), &frp);
    }

    if wp.w_vsep_width.get() == 0
        && frp.parent_mut().fr_layout.get() == FR_ROW
        && !frp.fr_prev.get().is_null()
    {
        frame_set_vsep(&frp.prev_mut(), true);
    }

    if frp.parent_mut().fr_layout.get() == FR_COL && !frp.fr_prev.get().is_null() {
        if global_stl_height() == 0 && wp.w_status_height.get() == 0 {
            frame_add_statusline(&frp.prev_mut());
        } else if global_stl_height() > 0 && wp.w_hsep_height.get() == 0 {
            frame_add_hsep(&frp.prev_mut());
        }
    }

    if dir == b'v' as i32 {
        let new_h = unflat_altfr.fr_height.get() - frp.fr_height.get();
        let is_next = frp.fr_next.get() == unflat_altfr;
        frame_new_height(unflat_altfr, new_h, is_next, false, false);
    } else if dir == b'h' as i32 {
        frame_new_width(
            unflat_altfr,
            unflat_altfr.fr_width.get() - frp.fr_width.get(),
            frp.fr_next.get() == unflat_altfr,
            false,
        );
    }

    if frp.fr_prev.get() != unflat_altfr {
        let topleft = frame2win(&frp.parent_mut());
        let mut row = topleft.w_winrow.get();
        let mut col = topleft.w_wincol.get();

        frame_comp_pos(&frp.parent_mut(), &mut row, &mut col);
    }
}

fn win_altframe(win: &WinInfoInfo, tp: Option<&TabPageInfo>) -> crate::globals::FramePtr {
    debug_assert!(tp.is_none_or(|t| g_st().curtab.get() != t));

    if one_window(win, tp) {
        return alt_tabpage().unwrap().curwin_mut().frame_mut();
    }

    let frp = win.frame_mut();

    if frp.fr_prev.get().is_null() {
        return frp.next_mut();
    }
    if frp.fr_next.get().is_null() {
        return frp.prev_mut();
    }

    let mut target_fr = frp.fr_next.get();
    let mut other_fr = frp.fr_prev.get();

    if !frp.fr_parent.get().is_null()
        && frp.parent_mut().fr_layout.get() == FR_COL
        && opt_vars().p_sb.get()
    {
        target_fr = frp.fr_prev.get();
        other_fr = frp.fr_next.get();
    }

    if !frp.fr_parent.get().is_null()
        && frp.parent_mut().fr_layout.get() == FR_ROW
        && opt_vars().p_spr.get()
    {
        target_fr = frp.fr_prev.get();
        other_fr = frp.fr_next.get();
    }

    if !frp.fr_parent.get().is_null() && frp.parent_mut().fr_layout.get() == FR_ROW {
        if frame_fixed_width(&target_fr) && !frame_fixed_width(&other_fr) {
            target_fr = other_fr;
        }
    } else if frame_fixed_height(&target_fr) && !frame_fixed_height(&other_fr) {
        target_fr = other_fr;
    }

    target_fr
}

fn alt_tabpage() -> Option<crate::globals::TabpagePtr> {
    if (opt_vars().tcl_flags.get() & K_OPT_TCL_FLAG_USELAST) != 0
        && valid_tabpage(g_st().lastused_tabpage.get().opt())
    {
        return Some(g_st().lastused_tabpage.get().clone());
    }

    let forward = !curtab_mut().tp_next.get().is_null()
        && ((opt_vars().tcl_flags.get() & K_OPT_TCL_FLAG_LEFT) == 0
            || g_st().curtab.get() == g_st().first_tabpage.get());

    if forward {
        curtab_mut().next_opt()
    } else {
        for tp in for_all_tabpages_ptr() {
            if tp.tp_next.get() == g_st().curtab.get() {
                return Some(tp);
            }
        }
        (!g_st().first_tabpage.get().is_null()).then(|| g_st().first_tabpage.get().clone())
    }
}

pub fn frame2win(frp: &Frame) -> crate::globals::WinPtr {
    let mut fr = crate::globals::FramePtr::from(frp);
    while fr.fr_win.get().is_null() {
        fr = fr.fr_child.get();
    }
    fr.fr_win.get()
}

fn frame_has_win(frp: &Frame, wp: Option<&WinInfoInfo>) -> bool {
    let wp_ptr = crate::globals::WinPtr::from(wp);
    if frp.fr_layout.get() == FR_LEAF {
        return frp.fr_win.get() == wp_ptr;
    }
    for p in frame_iter_mut(frp.fr_child.get()) {
        if frame_has_win(&p, wp) {
            return true;
        }
    }
    false
}

fn is_bottom_win(wp: &WinInfoInfo) -> bool {
    let mut fr = wp.frame_mut();
    while let Some(parent) = fr.parent_opt() {
        if parent.fr_layout.get() == FR_COL && !fr.fr_next.get().is_null() {
            return false;
        }
        fr = parent;
    }
    true
}

pub fn frame_new_height(topfrp: &Frame, mut height: i32, topfirst: bool, wfh: bool, set_ch: bool) {
    if topfrp.fr_parent.get().is_null() && set_ch {
        let new_ch = WIN_STATE
            .with(|s| s.min_set_ch.get())
            .max(opt_vars().p_ch.get() + OptInt::from(topfrp.fr_height.get() - height));
        if new_ch != opt_vars().p_ch.get() {
            let save_ch = WIN_STATE.with(|s| s.min_set_ch.get());
            set_option_value(K_OPT_CMDHEIGHT, integer_obj(new_ch), 0, true, None);
            WIN_STATE.with(|s| s.min_set_ch.set(save_ch));
        }
        height = (rows_avail() as i32).min(height);
    }
    if let Some(wp) = topfrp.win_opt() {
        if is_bottom_win(&wp) {
            wp.w_hsep_height.set(0);
        }
        win_new_height(
            &wp,
            height - wp.w_hsep_height.get() - wp.w_status_height.get(),
        );
    } else if topfrp.fr_layout.get() == FR_ROW {
        let mut frp: crate::globals::FramePtr;
        loop {
            frp = topfrp.fr_child.get();
            while !frp.is_null() {
                frame_new_height(frp.borrow(), height, topfirst, wfh, set_ch);
                if frp.fr_height.get() > height {
                    height = frp.fr_height.get();
                    break;
                }
                frp = frp.fr_next.get();
            }
            if frp.is_null() {
                break;
            }
        }
    } else {
        let mut frp = topfrp.fr_child.get();
        if wfh {
            while frame_fixed_height(frp.borrow()) {
                frp = frp.fr_next.get();
                if frp.is_null() {
                    return;
                }
            }
        }
        if !topfirst {
            while !frp.fr_next.get().is_null() {
                frp = frp.fr_next.get();
            }
            if wfh {
                while frame_fixed_height(frp.borrow()) {
                    frp = frp.fr_prev.get();
                }
            }
        }

        let mut extra_lines = height - topfrp.fr_height.get();
        if extra_lines < 0 {
            while !frp.is_null() {
                let h = frame_minheight(frp.borrow(), crate::globals::WinPtr::NULL);
                if frp.fr_height.get() + extra_lines < h {
                    extra_lines += frp.fr_height.get() - h;
                    frame_new_height(frp.borrow(), h, topfirst, wfh, set_ch);
                } else {
                    let new_h = frp.fr_height.get() + extra_lines;
                    frame_new_height(frp.borrow(), new_h, topfirst, wfh, set_ch);
                    break;
                }
                if topfirst {
                    loop {
                        frp = frp.fr_next.get();
                        if !(wfh && !frp.is_null() && frame_fixed_height(frp.borrow())) {
                            break;
                        }
                    }
                } else {
                    loop {
                        frp = frp.fr_prev.get();
                        if !(wfh && !frp.is_null() && frame_fixed_height(frp.borrow())) {
                            break;
                        }
                    }
                }
                if frp.is_null() {
                    height -= extra_lines;
                }
            }
        } else if extra_lines > 0 {
            let new_h = frp.fr_height.get() + extra_lines;
            frame_new_height(frp.borrow(), new_h, topfirst, wfh, set_ch);
        }
    }
    topfrp.fr_height.set(height);
}

fn frame_fixed_height(fr: &Frame) -> bool {
    if let Some(win) = fr.win_opt() {
        return win.w_onebuf_opt.wo_wfh.get();
    }
    if fr.fr_layout.get() == FR_ROW {
        return frame_iter_mut(fr.fr_child.get()).any(|child| frame_fixed_height(&child));
    }
    frame_iter_mut(fr.fr_child.get()).all(|child| frame_fixed_height(&child))
}

fn frame_fixed_width(fr: &Frame) -> bool {
    if let Some(win) = fr.win_opt() {
        return win.w_onebuf_opt.wo_wfw.get();
    }
    if fr.fr_layout.get() == FR_COL {
        return frame_iter_mut(fr.fr_child.get()).any(|child| frame_fixed_width(&child));
    }
    frame_iter_mut(fr.fr_child.get()).all(|child| frame_fixed_width(&child))
}

fn frame_add_statusline(frp: &Frame) {
    let mut fr = crate::globals::FramePtr::from(frp);
    if fr.fr_layout.get() == FR_LEAF {
        fr.win_mut().w_status_height.set(STATUS_HEIGHT);
    } else if fr.fr_layout.get() == FR_ROW {
        for child in frame_iter_ptr(fr.fr_child.get()) {
            frame_add_statusline(&child);
        }
    } else {
        debug_assert!(fr.fr_layout.get() == FR_COL);
        fr = fr.child_mut();
        while !fr.fr_next.get().is_null() {
            fr = fr.next_mut();
        }
        frame_add_statusline(&fr);
    }
}

fn frame_new_width(topfrp: &Frame, mut width: i32, leftfirst: bool, wfw: bool) {
    if topfrp.fr_layout.get() == FR_LEAF {
        let wp = topfrp.fr_win.get();
        let mut frp = crate::globals::FramePtr::from(topfrp);
        while let Some(parent) = frp.parent_opt() {
            if parent.fr_layout.get() == FR_ROW && !frp.fr_next.get().is_null() {
                break;
            }
            frp = parent;
        }
        if frp.fr_parent.get().is_null() {
            wp.borrow_mut().w_vsep_width.set(0);
        }
        let new_w = width - wp.borrow().w_vsep_width.get();
        win_new_width(wp.borrow_mut(), new_w);
    } else if topfrp.fr_layout.get() == FR_COL {
        let mut frp: crate::globals::FramePtr;
        loop {
            frp = topfrp.fr_child.get();
            while !frp.is_null() {
                frame_new_width(frp.borrow(), width, leftfirst, wfw);
                if frp.fr_width.get() > width {
                    width = frp.fr_width.get();
                    break;
                }
                frp = frp.fr_next.get();
            }
            if frp.is_null() {
                break;
            }
        }
    } else {
        let mut frp = topfrp.fr_child.get();
        if wfw {
            while frame_fixed_width(frp.borrow()) {
                frp = frp.fr_next.get();
                if frp.is_null() {
                    return;
                }
            }
        }
        if !leftfirst {
            while !frp.fr_next.get().is_null() {
                frp = frp.fr_next.get();
            }
            if wfw {
                while frame_fixed_width(frp.borrow()) {
                    frp = frp.fr_prev.get();
                }
            }
        }

        let mut extra_cols = width - topfrp.fr_width.get();
        if extra_cols < 0 {
            while !frp.is_null() {
                let old_width = frp.fr_width.get();
                let new_w = (old_width + extra_cols)
                    .max(frame_minwidth(frp.borrow(), crate::globals::WinPtr::NULL));
                frame_new_width(frp.borrow(), new_w, leftfirst, wfw);
                // Take the rest from the next frame. Count the width actually taken: a frame may stay wider
                // than requested, e.g. if it contains a 'winfixwidth' window.
                extra_cols += old_width - frp.fr_width.get();
                if extra_cols >= 0 {
                    break;
                }
                if leftfirst {
                    loop {
                        frp = frp.fr_next.get();
                        if !(wfw && !frp.is_null() && frame_fixed_width(frp.borrow())) {
                            break;
                        }
                    }
                } else {
                    loop {
                        frp = frp.fr_prev.get();
                        if !(wfw && !frp.is_null() && frame_fixed_width(frp.borrow())) {
                            break;
                        }
                    }
                }
                if frp.is_null() {
                    width -= extra_cols;
                }
            }
        } else if extra_cols > 0 {
            let new_w = frp.fr_width.get() + extra_cols;
            frame_new_width(frp.borrow(), new_w, leftfirst, wfw);
        }
    }
    topfrp.fr_width.set(width);
}

fn frame_set_vsep(frp: &Frame, add: bool) {
    let mut fr = crate::globals::FramePtr::from(frp);
    if fr.fr_layout.get() == FR_LEAF {
        let wp = fr.win_mut();
        if add && wp.w_vsep_width.get() == 0 {
            if wp.w_width.get() > 0 {
                win_new_width(&wp, wp.w_width.get() - 1);
            }
            wp.w_vsep_width.set(1);
        } else if !add && wp.w_vsep_width.get() == 1 {
            win_new_width(&wp, wp.w_width.get() + 1);
            wp.w_vsep_width.set(0);
        }
    } else if fr.fr_layout.get() == FR_COL {
        for child in frame_iter_ptr(fr.fr_child.get()) {
            frame_set_vsep(&child, add);
        }
    } else {
        debug_assert!(fr.fr_layout.get() == FR_ROW);
        fr = fr.child_mut();
        while !fr.fr_next.get().is_null() {
            fr = fr.next_mut();
        }
        frame_set_vsep(&fr, add);
    }
}

fn frame_add_hsep(frp: &Frame) {
    let mut fr = crate::globals::FramePtr::from(frp);
    if fr.fr_layout.get() == FR_LEAF {
        fr.win_mut().w_hsep_height.set(1);
    } else if fr.fr_layout.get() == FR_ROW {
        for child in frame_iter_ptr(fr.fr_child.get()) {
            frame_add_hsep(&child);
        }
    } else {
        debug_assert!(fr.fr_layout.get() == FR_COL);
        fr = fr.child_mut();
        while !fr.fr_next.get().is_null() {
            fr = fr.next_mut();
        }
        frame_add_hsep(&fr);
    }
}

#[inline]
fn frame_fix_width(wp: &WinInfoInfo) {
    wp.frame_mut()
        .fr_width
        .set(wp.w_width.get() + wp.w_vsep_width.get());
}

#[inline]
fn frame_fix_height(wp: &WinInfoInfo) {
    wp.frame_mut()
        .fr_height
        .set(wp.w_height.get() + wp.w_hsep_height.get() + wp.w_status_height.get());
}

fn frame_minheight(topfrp: &Frame, next_curwin: impl Into<Option<crate::globals::WinPtr>>) -> i32 {
    let next_curwin = next_curwin.into();
    let mut m: i32;
    if let Some(win) = topfrp.win_opt() {
        let extra_height =
            win.w_winbar_height.get() + win.w_hsep_height.get() + win.w_status_height.get();
        if next_curwin
            .as_ref()
            .is_some_and(|nc| topfrp.fr_win.get() == *nc)
        {
            m = opt_vars().p_wh.get() as i32 + extra_height;
        } else {
            m = opt_vars().p_wmh.get() as i32 + extra_height;
            if topfrp.fr_win.get() == curwin_ptr()
                && next_curwin
                    .as_ref()
                    .is_some_and(crate::globals::WinPtr::is_null)
                && opt_vars().p_wmh.get() == 0
            {
                m += 1;
            }
        }
    } else if topfrp.fr_layout.get() == FR_ROW {
        m = 0;
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            let n = frame_minheight(&frp, next_curwin.clone());
            if n > m {
                m = n;
            }
        }
    } else {
        m = 0;
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            m += frame_minheight(&frp, next_curwin.clone());
        }
    }
    m
}

fn frame_minwidth(topfrp: &Frame, next_curwin: impl Into<Option<crate::globals::WinPtr>>) -> i32 {
    let next_curwin = next_curwin.into();
    let mut m: i32;
    if let Some(win) = topfrp.win_opt() {
        if next_curwin
            .as_ref()
            .is_some_and(|nc| topfrp.fr_win.get() == *nc)
        {
            m = opt_vars().p_wiw.get() as i32 + win.w_vsep_width.get();
        } else {
            m = opt_vars().p_wmw.get() as i32 + win.w_vsep_width.get();
            if opt_vars().p_wmw.get() == 0
                && topfrp.fr_win.get() == curwin_ptr()
                && next_curwin
                    .as_ref()
                    .is_some_and(crate::globals::WinPtr::is_null)
            {
                m += 1;
            }
        }
    } else if topfrp.fr_layout.get() == FR_COL {
        m = 0;
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            let n = frame_minwidth(&frp, next_curwin.clone());
            m = m.max(n);
        }
    } else {
        m = 0;
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            m += frame_minwidth(&frp, next_curwin.clone());
        }
    }
    m
}

pub fn close_others(message: i32, forceit: bool, ignore_pinned: bool) {
    let old_curwin = curwin_ptr();

    if curwin_mut().w_floating.get() {
        if message != 0 && !g_st().autocmd_busy.get() {
            emsg(E_FLOATONLY);
        }
        return;
    }

    if one_window(&firstwin_mut(), None) && !lastwin_mut().w_floating.get() {
        if message != 0 && !g_st().autocmd_busy.get() {
            msg(gettext(M_ONLYONE), 0);
        }
        return;
    }

    let mut nextwp: crate::globals::WinPtr;
    let mut wp = g_st().firstwin.get();
    while win_valid(wp.opt()) {
        nextwp = wp.borrow().w_next.get();

        if old_curwin != curwin_ptr() && win_valid(old_curwin.opt()) {
            set_curwin(&old_curwin);
            g_st().curbuf.set(curwin_mut().w_buffer.get());
        }

        if wp == curwin_handle() || (wp.borrow().w_onebuf_opt.wo_wp.get() && !ignore_pinned) {
            wp = nextwp;
            continue;
        }

        if !buf_valid(wp.borrow().buf_opt().as_deref()) && win_valid(wp.opt()) {
            wp.borrow_mut().w_buffer.set(crate::globals::BufPtr::NULL);
            win_close(wp.borrow_mut(), false, false);
            wp = nextwp;
            continue;
        }

        let r = can_abandon(&wp.borrow().buf_mut(), forceit);
        if !win_valid(wp.opt()) {
            wp = g_st().firstwin.get();
            continue;
        }
        if !r {
            if message != 0
                && (opt_vars().p_confirm.get()
                    || (g_st().cmdmod.borrow().cmod_flags & CMOD_CONFIRM) != 0)
                && opt_vars().p_write.get()
            {
                dialog_changed(&wp.borrow().buf_mut(), false);
                if !win_valid(wp.opt()) {
                    wp = g_st().firstwin.get();
                    continue;
                }
            }
            if buf_is_changed_fn(&wp.borrow().buf_mut()) {
                wp = nextwp;
                continue;
            }
        }
        let close_buf =
            !buf_hide(&wp.borrow().buf_mut()) && !buf_is_changed_fn(&wp.borrow().buf_mut());
        win_close(wp.borrow_mut(), close_buf, false);
        wp = nextwp;
    }

    if message != 0 && !one_window_macro() {
        let mut has_non_pinned = false;
        for wp2 in for_all_windows_ptr() {
            if wp2 != curwin_ptr() && !wp2.w_onebuf_opt.wo_wp.get() {
                has_non_pinned = true;
                break;
            }
        }
        if has_non_pinned {
            emsg(gettext(c"E445: Other window contains changes"));
        }
    }
}

pub fn unuse_tabpage(tp: &TabPageInfo) {
    tp.tp_topframe.set(g_st().topframe.get().clone());
    tp.tp_firstwin.set(g_st().firstwin.get());
    tp.tp_lastwin.set(g_st().lastwin.get());
    tp.tp_curwin.set(crate::globals::curwin_ptr());
    tp.tp_ch_used.set(opt_vars().p_ch.get());
}

pub fn use_tabpage(tp: &TabPageInfo) {
    g_st().curtab.set(crate::globals::TabpagePtr::from(tp));
    g_st().topframe.set(curtab_mut().tp_topframe.get());
    g_st().firstwin.set(curtab_mut().tp_firstwin.get());
    g_st().lastwin.set(curtab_mut().tp_lastwin.get());
    set_curwin(curtab_mut().tp_curwin.get());
    opt_vars().p_ch.set(curtab_mut().tp_ch_used.get());
}

pub fn win_alloc_first() {
    if !win_alloc_firstwin(None) {
        std::process::abort();
    }

    g_st().first_tabpage.set(alloc_tabpage());
    g_st().curtab.set(g_st().first_tabpage.get().clone());
    unuse_tabpage(&crate::globals::first_tabpage_mut());
}

pub fn win_alloc_ctx_win(idx: i32) {
    let mut err = ERROR_INIT;
    let mut fconfig = WIN_CONFIG_INIT;
    fconfig.width = g_st().columns_global.get();
    fconfig.height = 5;
    fconfig.focusable = false;
    fconfig.mouse = false;
    fconfig.hide = true;
    let cw_ref = win_new_float(None, true, fconfig, &mut err).unwrap();
    g_st().ctx_win_vec.borrow_mut().as_mut_slice()[idx as usize].cw_win = cw_ref.clone();
    cw_ref
        .buf_mut()
        .b_nwindows
        .set(cw_ref.buf_mut().b_nwindows.get() - (1));
    reset_binding(&cw_ref);
}

fn win_alloc_firstwin(oldwin: Option<&WinInfoInfo>) -> bool {
    set_curwin(win_alloc(None, false));
    if let Some(oldwin) = oldwin {
        win_init(&curwin_mut(), oldwin, 0);
        reset_binding(&curwin_mut());
    } else {
        g_st().curbuf.set(crate::globals::BufPtr::from(buflist_new(
            None, None, 1, BLN_LISTED,
        )));
        if g_st().curbuf.get().is_null() {
            return false;
        }
        curwin_mut().w_buffer.set(g_st().curbuf.get());
        curwin_mut().w_s_owned.set(None);
        curbuf_mut().b_nwindows.set(1);
        curwin_mut().w_alist.set(None);
        curwin_init();
    }

    new_frame(&curwin_mut());
    g_st().topframe.set(curwin_mut().w_frame.get().clone());
    crate::globals::topframe_mut()
        .fr_width
        .set(g_st().columns_global.get());
    crate::globals::topframe_mut()
        .fr_height
        .set(g_st().rows_global.get() - opt_vars().p_ch.get() as i32 - global_stl_height());

    true
}

fn new_frame(wp: &WinInfoInfo) {
    let fr = Frame::new_rc();
    fr.fr_layout.set(FR_LEAF);
    fr.fr_win.set(crate::globals::WinPtr::from(wp));
    wp.w_frame.set(crate::globals::FramePtr::from(fr));
}

pub fn win_init_size() {
    let avail = rows_avail() as i32;
    firstwin_mut().w_height.set(avail);
    firstwin_mut().w_prev_height.set(avail);
    firstwin_mut()
        .w_view_height
        .set(firstwin_mut().w_height.get() - firstwin_mut().w_winbar_height.get());
    firstwin_mut()
        .w_height_outer
        .set(firstwin_mut().w_height.get());
    firstwin_mut()
        .w_winrow_off
        .set(firstwin_mut().w_winbar_height.get());
    crate::globals::topframe_mut().fr_height.set(avail);
    firstwin_mut().w_width.set(g_st().columns_global.get());
    firstwin_mut()
        .w_view_width
        .set(firstwin_mut().w_width.get());
    firstwin_mut()
        .w_width_outer
        .set(firstwin_mut().w_width.get());
    crate::globals::topframe_mut()
        .fr_width
        .set(g_st().columns_global.get());
}

fn alloc_tabpage() -> crate::globals::TabpagePtr {
    WIN_STATE.with(|s| s.last_tp_handle.set(s.last_tp_handle.get() + (1)));
    let handle = WIN_STATE.with(|s| s.last_tp_handle.get());
    let tp = crate::globals::TabpagePtr::from(TabPageInfo::new_rc(handle));
    g_st()
        .tabpage_handles
        .borrow_mut()
        .insert(handle, tp.clone());

    let winvar = crate::eval::typval::tv_dict_item_alloc_static();
    init_var_dict(&tv_dict_alloc(), &winvar, VAR_SCOPE);
    tp.tp_winvar.set(winvar);
    tp.tp_ch_used.set(opt_vars().p_ch.get());

    tp
}

impl Drop for TabPageInfo {
    fn drop(&mut self) {
        if let Some(tp_winvar) = self.tp_winvar.get().opt() {
            if !tp_winvar.tv().vval.v_dict().is_null() {
                vars_clear(&self.tp_vars().dv_hashtab);
                unref_var_dict(&self.tp_vars());
            }
            crate::eval::typval::tv_dict_item_free_slot(tp_winvar);
            self.tp_winvar.set(crate::eval::typval::DictitemId::NULL);
        }
    }
}

pub fn free_tabpage(tp: Option<&TabPageInfo>) {
    if let Some(tp_ref) = tp {
        g_st().tabpage_handles.borrow_mut().remove(&tp_ref.handle);
        diff_clear(tp_ref);
        for idx in 0..SNAP_COUNT {
            clear_snapshot(tp_ref, idx as i32);
        }
        if let Some(tp_winvar) = tp_ref.tp_winvar.get().opt() {
            if !tp_winvar.tv().vval.v_dict().is_null() {
                vars_clear(&tp_ref.tp_vars().dv_hashtab);
                unref_var_dict(&tp_ref.tp_vars());
            }
            crate::eval::typval::tv_dict_item_free_slot(tp_winvar);
            tp_ref.tp_winvar.set(crate::eval::typval::DictitemId::NULL);
        }
        if g_st().lastused_tabpage.get() == tp_ref {
            g_st()
                .lastused_tabpage
                .set(crate::globals::TabpagePtr::NULL);
        }
        tp_ref.tp_next.set(crate::globals::TabpagePtr::NULL);
        tp_ref.tp_topframe.set(crate::globals::FramePtr::NULL);
    }
}

pub fn win_new_tabpage(
    after: i32,
    filename_opt: Option<&CStr>,
    enter: bool,
    first: Option<&mut Option<crate::globals::WinPtr>>,
) -> Option<crate::globals::TabpagePtr> {
    {
        let old_curtab = g_st().curtab.get().clone();

        if window_layout_locked(Cmdidx::CmdTabnew) {
            return None;
        }

        let newtp = alloc_tabpage();

        if enter {
            if !leave_tabpage(g_st().curbuf.get().opt_mut(), true) {
                free_tabpage(newtp.opt());
                return None;
            }
        } else {
            unuse_tabpage(&curtab_mut());
            curtab_mut().tp_old_rows_avail.set(rows_avail());
            g_st().firstwin.set(crate::globals::WinPtr::NULL);
            g_st().lastwin.set(crate::globals::WinPtr::NULL);
        }

        newtp.tp_localdir.set(old_curtab.tp_localdir.get());

        g_st().curtab.set(newtp.clone());

        let result = win_alloc_firstwin(old_curtab.tp_curwin.get().opt_mut());
        debug_assert!(result);
        if let Some(first_out) = first {
            let cw = curwin_ptr();
            *first_out = (!cw.is_null()).then_some(cw);
        }

        if after == 1 {
            newtp.tp_next.set(g_st().first_tabpage.get().clone());
            g_st().first_tabpage.set(newtp.clone());
        } else {
            let mut tp = old_curtab.clone();
            if after > 0 {
                let mut n: i32 = 2;
                tp = g_st().first_tabpage.get().clone();
                while !tp.tp_next.get().is_null() && n < after {
                    n += 1;
                    tp = tp.tp_next.get();
                }
            }
            newtp.tp_next.set(tp.tp_next.get());
            tp.tp_next.set(newtp.clone());
        }
        newtp.tp_firstwin.set(crate::globals::curwin_ptr());
        newtp.tp_lastwin.set(crate::globals::curwin_ptr());
        newtp.tp_curwin.set(crate::globals::curwin_ptr());

        win_init_size();
        firstwin_mut().w_winrow.set(tabline_height());
        firstwin_mut()
            .w_prev_winrow
            .set(firstwin_mut().w_winrow.get());
        win_comp_scroll(&curwin_mut());

        newtp.tp_topframe.set(g_st().topframe.get().clone());
        last_status(false);

        if !curbuf_mut().terminal.get().is_null() {
            terminal_check_size(curbuf_mut().terminal_mut());
        }

        if enter {
            redraw_all_later(UPD_NOT_VALID);
            tabpage_check_windows(old_curtab.borrow());
            g_st().lastused_tabpage.set(old_curtab);
            entering_window(&curwin_mut());

            apply_autocmds(EVENT_WINNEW, None, None, false, curbuf_opt_mut().as_deref());
            apply_autocmds(
                EVENT_WINENTER,
                None,
                None,
                false,
                curbuf_opt_mut().as_deref(),
            );
            apply_autocmds(
                EVENT_TABNEW,
                filename_opt,
                filename_opt,
                false,
                curbuf_opt_mut().as_deref(),
            );
            apply_autocmds(
                EVENT_TABENTER,
                None,
                None,
                false,
                curbuf_opt_mut().as_deref(),
            );
        } else {
            unuse_tabpage(&curtab_mut());
            use_tabpage(old_curtab.borrow());
            g_st().redraw_tabline.set(true);
            if curtab_mut().tp_old_rows_avail.get() != rows_avail() {
                win_new_screen_rows();
            }

            let mut switchwin: CtxSwitch = CtxSwitch::new();
            let sw_ok = ctx_switch(
                &mut switchwin,
                newtp.tp_curwin.get().opt_mut(),
                newtp.opt(),
                None,
                K_CTX_NO_DISPLAY,
            );
            debug_assert!(sw_ok);

            apply_autocmds(EVENT_WINNEW, None, None, false, curbuf_opt_mut().as_deref());
            apply_autocmds(
                EVENT_TABNEW,
                filename_opt,
                filename_opt,
                false,
                curbuf_opt_mut().as_deref(),
            );

            ctx_restore(&mut switchwin);
        }

        Some(newtp)
    }
}

fn may_open_tabpage() -> bool {
    let n = if g_st().cmdmod.borrow().cmod_tab == 0 {
        g_st().postponed_split_tab.get()
    } else {
        g_st().cmdmod.borrow().cmod_tab
    };

    if n == 0 {
        return false;
    }

    g_st().cmdmod.borrow_mut().cmod_tab = 0;
    g_st().postponed_split_tab.set(0);
    let status = win_new_tabpage(n, None, true, None).is_some();
    if status {
        apply_autocmds(
            EVENT_TABNEWENTERED,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
    }
    status
}

pub fn make_tabpages(maxcount: i32) -> i32 {
    let count = maxcount.min(opt_vars().p_tpm.get() as i32);

    block_autocmds();

    let mut todo = count - 1;
    while todo > 0 {
        if win_new_tabpage(0, None, true, None).is_none() {
            break;
        }
        todo -= 1;
    }

    unblock_autocmds();

    count - todo
}

pub fn valid_tabpage(tpc: Option<&TabPageInfo>) -> bool {
    let Some(tpc_ref) = tpc else {
        return false;
    };
    let tpc_ptr = TabpagePtr::from(tpc_ref);
    let mut tp = g_st().first_tabpage.get().clone();
    while let Some(t) = tp.opt() {
        if tp == tpc_ptr {
            return true;
        }
        tp = t.tp_next.get();
    }
    false
}

pub fn valid_tabpage_win(tpc: &TabPageInfo) -> i32 {
    for tp in for_all_tabpages_ptr() {
        if tp == tpc {
            for wp in win_iter_ptr(if g_st().curtab.get() == tp {
                g_st().firstwin.get()
            } else {
                tp.tp_firstwin.get()
            }) {
                if win_valid_any_tab(Some(&wp)) {
                    return 1;
                }
            }
            return 0;
        }
    }
    0
}

pub fn close_tabpage(tab: &TabPageInfo) {
    let tab_ptr = crate::globals::TabpagePtr::from(tab);
    let ptp = if g_st().first_tabpage.get() == tab_ptr {
        g_st().first_tabpage.set(tab_ptr.tp_next.get());
        g_st().first_tabpage.get().clone()
    } else {
        let mut prev_tp = crate::globals::TabpagePtr::NULL;
        for tp in for_all_tabpages_mut() {
            if tp.tp_next.get() == tab_ptr {
                prev_tp = tp;
                break;
            }
        }
        assert!(!prev_tp.is_null(), "tabpage in list");
        prev_tp.tp_next.set(tab_ptr.tp_next.get());
        prev_tp
    };

    goto_tabpage_tp(ptp.opt(), false, false);
    free_tabpage(tab_ptr.opt());
}

pub fn find_tabpage(n: i32) -> Option<crate::globals::TabpagePtr> {
    if n == 0 {
        return (!g_st().curtab.get().is_null()).then(|| g_st().curtab.get().clone());
    }
    if n < 1 {
        return None;
    }
    for_all_tabpages_ptr().nth((n - 1) as usize)
}

pub fn tabpage_index(ftp: Option<&TabPageInfo>) -> i32 {
    let ftp_ptr = crate::globals::TabpagePtr::from(ftp);
    let mut i: i32 = 1;
    for tp in for_all_tabpages_ptr() {
        if ftp_ptr == tp {
            break;
        }
        i += 1;
    }
    i
}

fn leave_tabpage(new_curbuf_in: Option<&FileBuf>, trigger_leave_autocmds: bool) -> bool {
    let new_curbuf = crate::globals::BufPtr::from(new_curbuf_in);
    let tp_ptr = curtab_mut();

    leaving_window(&curwin_mut());
    reset_visual_and_resel();
    if trigger_leave_autocmds {
        if g_st().curbuf.get() != new_curbuf {
            apply_autocmds(
                EVENT_BUFLEAVE,
                None,
                None,
                false,
                curbuf_opt_mut().as_deref(),
            );
            if g_st().curtab.get() != tp_ptr {
                return false;
            }
        }
        apply_autocmds(
            EVENT_WINLEAVE,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
        if g_st().curtab.get() != tp_ptr {
            return false;
        }
        apply_autocmds(
            EVENT_TABLEAVE,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
        if g_st().curtab.get() != tp_ptr {
            return false;
        }
    }

    let tp = tp_ptr.borrow();
    reset_dragwin();
    tp.tp_curwin.set(crate::globals::curwin_ptr());
    tp.tp_prevwin.set(g_st().prevwin.get());
    tp.tp_firstwin.set(g_st().firstwin.get());
    tp.tp_lastwin.set(g_st().lastwin.get());
    tp.tp_old_rows_avail.set(rows_avail());
    if tp.tp_old_columns.get() != -1 {
        tp.tp_old_columns
            .set(i64::from(g_st().columns_global.get()));
    }
    g_st().firstwin.set(crate::globals::WinPtr::NULL);
    g_st().lastwin.set(crate::globals::WinPtr::NULL);
    true
}

fn enter_tabpage(
    tp_in: &TabPageInfo,
    old_curbuf_in: Option<&FileBuf>,
    trigger_enter_autocmds: bool,
    trigger_leave_autocmds: bool,
) {
    let tp_ptr = crate::globals::TabpagePtr::from(tp_in);
    let old_curbuf = crate::globals::BufPtr::from(old_curbuf_in);
    let tp = tp_ptr.borrow();
    let old_off = tp.firstwin_mut().w_winrow.get();
    let next_prevwin = tp.tp_prevwin.get();
    let old_curtab_ptr = curtab_mut();
    let prev_p_ch = opt_vars().p_ch.get();

    use_tabpage(tp);

    if g_st().curtab.get() != old_curtab_ptr && opt_vars().p_ch.get() != prev_p_ch {
        tabpage_check_windows(old_curtab_ptr.borrow());
        let new_ch = opt_vars().p_ch.get();
        opt_vars().p_ch.set(prev_p_ch);
        WIN_STATE.with(|s| s.command_frame_height.set(false));
        set_option_value(K_OPT_CMDHEIGHT, integer_obj(new_ch), 0, true, None);
        WIN_STATE.with(|s| s.command_frame_height.set(true));
    } else if g_st().curtab.get() != old_curtab_ptr {
        tabpage_check_windows(old_curtab_ptr.borrow());
    }

    let tp_curwin = tp_ptr.tp_curwin.get();
    win_enter_ext(
        tp_curwin.borrow_mut(),
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
    g_st().prevwin.set(next_prevwin);

    last_status(false);
    win_float_update_statusline();
    win_comp_pos();
    g_st().diff_need_scrollbind.set(true);

    reset_dragwin();

    if curtab_mut().tp_old_rows_avail.get() != rows_avail()
        || old_off != firstwin_mut().w_winrow.get()
    {
        win_new_screen_rows();
    }
    if curtab_mut().tp_old_columns.get() != i64::from(g_st().columns_global.get()) {
        if starting_val() == 0 {
            win_new_screen_cols();
            curtab_mut()
                .tp_old_columns
                .set(i64::from(g_st().columns_global.get()));
        } else {
            curtab_mut().tp_old_columns.set(-1);
        }
    }

    g_st().lastused_tabpage.set(old_curtab_ptr);

    if trigger_enter_autocmds {
        apply_autocmds(
            EVENT_TABENTER,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
        if g_st().curbuf.get() != old_curbuf {
            apply_autocmds(
                EVENT_BUFENTER,
                None,
                None,
                false,
                curbuf_opt_mut().as_deref(),
            );
        }
    }

    redraw_all_later(UPD_NOT_VALID);
}

fn tabpage_check_windows(old_curtab: &TabPageInfo) {
    for wp in win_iter_ptr(old_curtab.tp_firstwin.get()) {
        if wp.w_floating.get() {
            if wp.w_config.borrow().external {
                win_remove(&wp, Some(old_curtab));
                win_append(Some(&lastwin_nofloating(None)), &wp, None);
            } else {
                ui_comp_remove_grid(&wp.w_grid_alloc);
            }
        }
        wp.w_pos_changed.set(true);
    }

    for wp2 in for_all_windows_mut() {
        let is_non_ext_float = wp2.w_floating.get() && !wp2.w_config.borrow().external;
        if is_non_ext_float {
            let cfg = wp2.w_config.borrow().clone();
            win_config_float(&wp2, cfg);
        }
        wp2.w_pos_changed.set(true);
    }
}

pub fn goto_tabpage(n: i32) {
    if text_locked() {
        text_locked_msg();
        return;
    }

    if crate::globals::first_tabpage_mut().tp_next.get().is_null() {
        if n > 1 {
            beep_flush();
        }
        return;
    }

    let mut tp = crate::globals::TabpagePtr::NULL;

    if n == 0 {
        if curtab_mut().tp_next.get().is_null() {
            tp = g_st().first_tabpage.get().clone();
        } else {
            tp = curtab_mut().tp_next.get();
        }
    } else if n < 0 {
        let mut ttp = g_st().curtab.get().clone();
        for _ in n..0 {
            let mut cur = crate::globals::first_tabpage_mut();
            while cur.tp_next.get() != ttp
                && let Some(next) = cur.next_opt()
            {
                cur = next;
            }
            tp = cur;
            ttp = tp.clone();
        }
    } else if n == 9999 {
        if let Some(last) = for_all_tabpages_ptr().last() {
            tp = last;
        }
    } else {
        tp = crate::globals::TabpagePtr::from(find_tabpage(n));
        if tp.is_null() {
            beep_flush();
            return;
        }
    }

    goto_tabpage_tp(tp.opt(), true, true);
}

pub fn goto_tabpage_tp(
    tp_in: Option<&TabPageInfo>,
    trigger_enter_autocmds: bool,
    trigger_leave_autocmds: bool,
) {
    let tp = crate::globals::TabpagePtr::from(tp_in);
    set_keep_msg(None, 0);

    g_st().skip_win_fix_scroll.set(true);
    let Some(tp_ref) = tp.opt() else {
        g_st().skip_win_fix_scroll.set(false);
        return;
    };
    let new_curbuf = tp_ref.curwin_mut().w_buffer.get();
    if g_st().curtab.get() != tp && leave_tabpage(new_curbuf.opt_mut(), trigger_leave_autocmds) {
        if valid_tabpage(tp.opt()) {
            enter_tabpage(
                tp.borrow(),
                g_st().curbuf.get().opt_mut(),
                trigger_enter_autocmds,
                trigger_leave_autocmds,
            );
        } else {
            enter_tabpage(
                &curtab_mut(),
                g_st().curbuf.get().opt_mut(),
                trigger_enter_autocmds,
                trigger_leave_autocmds,
            );
        }
    }
    g_st().skip_win_fix_scroll.set(false);
}

pub fn goto_tabpage_lastused() -> bool {
    if !valid_tabpage(g_st().lastused_tabpage.get().opt()) {
        return false;
    }

    let last = g_st().lastused_tabpage.get().clone();
    goto_tabpage_tp(last.opt(), true, true);
    true
}

pub fn goto_tabpage_win(tp_in: &TabPageInfo, wp_in: &WinInfoInfo) {
    let tp = crate::globals::TabpagePtr::from(tp_in);
    let wp = crate::globals::WinPtr::from(wp_in);
    goto_tabpage_tp(tp.opt(), true, true);
    if g_st().curtab.get() == tp && win_valid(wp.opt()) {
        win_enter(wp.borrow_mut(), true);
    }
}

pub fn tabpage_move(nr: i32) {
    debug_assert!(!g_st().curtab.get().is_null());

    if crate::globals::first_tabpage_mut().tp_next.get().is_null() {
        return;
    }

    if g_st().tabpage_move_disallowed.get() != 0 {
        return;
    }

    let mut n: i32 = 1;
    let mut tp = crate::globals::first_tabpage_mut();
    while n < nr
        && let Some(next) = tp.next_opt()
    {
        n += 1;
        tp = next;
    }

    if g_st().curtab.get() == tp
        || (nr > 0 && !tp.tp_next.get().is_null() && tp.tp_next.get() == g_st().curtab.get())
    {
        return;
    }

    let old_nr = tabpage_index(g_st().curtab.get().opt());
    let tp_dst = tp;

    if g_st().curtab.get() == g_st().first_tabpage.get() {
        g_st().first_tabpage.set(curtab_mut().tp_next.get());
    } else {
        let Some(prev_tp) =
            for_all_tabpages_ptr().find(|tp2| tp2.tp_next.get() == g_st().curtab.get())
        else {
            return;
        };
        prev_tp.tp_next.set(curtab_mut().tp_next.get());
    }

    if nr <= 0 {
        curtab_mut().tp_next.set(g_st().first_tabpage.get().clone());
        g_st().first_tabpage.set(g_st().curtab.get().clone());
    } else {
        curtab_mut().tp_next.set(tp_dst.tp_next.get());
        tp_dst.tp_next.set(g_st().curtab.get().clone());
    }

    g_st().redraw_tabline.set(true);

    if has_event(EVENT_TABMOVED) {
        let mut prev_idx = [0u8; NUMBUFLEN];
        format_to_buf!(&mut prev_idx[..], c"%i", old_nr);
        let mut data_items = [
            KeyValuePair {
                key: cstr_as_string(Some(c"tabnr_old")),
                value: integer_obj(Integer::from(old_nr)),
            },
            KeyValuePair {
                key: cstr_as_string(Some(c"tabnr_new")),
                value: integer_obj(Integer::from(tabpage_index(g_st().curtab.get().opt()))),
            },
        ];
        let data = Dict::from_slice(&mut data_items[..2]);
        let mut obj = dict_obj(data);
        aucmd_defer(
            EVENT_TABMOVED,
            Some(crate::memory::bytes_as_cstr(&prev_idx)),
            None,
            AUGROUP_ALL,
            &curbuf_mut(),
            Some(&mut obj),
        );
    }
}

pub fn win_goto(wp_in: &WinInfoInfo) {
    let wp = crate::globals::WinPtr::from(wp_in);
    if text_or_buf_locked() {
        beep_flush();
        return;
    }

    let wp_ref = wp.borrow_mut();
    if g_st().curbuf.get() != wp_ref.w_buffer.get() {
        reset_visual_and_resel();
    } else if g_st().visual_global.borrow().active {
        wp_ref.w_cursor.set(curwin_mut().w_cursor.get());
    }

    if !win_valid(wp.opt()) {
        return;
    }

    win_enter(wp.borrow_mut(), true);
}

pub fn win_find_tabpage(win: &WinInfoInfo) -> Option<crate::globals::TabpagePtr> {
    for tp in for_all_tabpages_mut() {
        for wp in for_all_windows_in_tab_ptr(&tp) {
            if wp == win {
                return Some(tp);
            }
        }
    }
    None
}

pub fn win_vert_neighbor(
    tp: &TabPageInfo,
    wp: &WinInfoInfo,
    up: bool,
    mut count: i32,
) -> Option<crate::globals::WinPtr> {
    if wp.w_floating.get() {
        let res = if win_valid(g_st().prevwin.get().opt())
            && !crate::globals::prevwin_mut().w_floating.get()
        {
            g_st().prevwin.get()
        } else {
            g_st().firstwin.get()
        };
        return (!res.is_null()).then_some(res);
    }

    let mut foundfr = wp.frame_mut();

    'outer: while count > 0 {
        count -= 1;
        let mut nfr: crate::globals::FramePtr;
        let mut fr = foundfr.clone();
        loop {
            if tp.tp_topframe.get() == fr {
                break 'outer;
            }
            let cand = if up { fr.prev_opt() } else { fr.next_opt() };
            if fr.parent_mut().fr_layout.get() == FR_COL
                && let Some(c) = cand
            {
                nfr = c;
                break;
            }
            fr = fr.parent_mut();
        }

        loop {
            if nfr.fr_layout.get() == FR_LEAF {
                foundfr = nfr;
                break;
            }
            fr = nfr.child_mut();
            if nfr.fr_layout.get() == FR_ROW {
                while frame2win(&fr).w_wincol.get() + fr.fr_width.get()
                    <= wp.w_wincol.get() + wp.w_wcol.get()
                    && let Some(next) = fr.next_opt()
                {
                    fr = next;
                }
            }
            if nfr.fr_layout.get() == FR_COL && up {
                while let Some(next) = fr.next_opt() {
                    fr = next;
                }
            }
            nfr = fr;
        }
    }
    let w = foundfr.fr_win.get();
    (!w.is_null()).then_some(w)
}

fn win_goto_ver(up: bool, count: i32) {
    if let Some(win) = win_vert_neighbor(&curtab_mut(), &curwin_mut(), up, count) {
        win_goto(&win);
    }
}

pub fn win_horz_neighbor(
    tp: &TabPageInfo,
    wp: &WinInfoInfo,
    left: bool,
    mut count: i32,
) -> Option<crate::globals::WinPtr> {
    if wp.w_floating.get() {
        let res = if win_valid(g_st().prevwin.get().opt())
            && !crate::globals::prevwin_mut().w_floating.get()
        {
            g_st().prevwin.get()
        } else {
            g_st().firstwin.get()
        };
        return (!res.is_null()).then_some(res);
    }

    let mut foundfr = wp.frame_mut();

    'outer: while count > 0 {
        count -= 1;
        let mut nfr: crate::globals::FramePtr;
        let mut fr = foundfr.clone();
        loop {
            if tp.tp_topframe.get() == fr {
                break 'outer;
            }
            let cand = if left { fr.prev_opt() } else { fr.next_opt() };
            if fr.parent_mut().fr_layout.get() == FR_ROW
                && let Some(c) = cand
            {
                nfr = c;
                break;
            }
            fr = fr.parent_mut();
        }

        loop {
            if nfr.fr_layout.get() == FR_LEAF {
                foundfr = nfr;
                break;
            }
            fr = nfr.child_mut();
            if nfr.fr_layout.get() == FR_COL {
                while frame2win(&fr).w_winrow.get() + fr.fr_height.get()
                    <= wp.w_winrow.get() + wp.w_wrow.get()
                    && let Some(next) = fr.next_opt()
                {
                    fr = next;
                }
            }
            if nfr.fr_layout.get() == FR_ROW && left {
                while let Some(next) = fr.next_opt() {
                    fr = next;
                }
            }
            nfr = fr;
        }
    }
    let w = foundfr.fr_win.get();
    (!w.is_null()).then_some(w)
}

fn win_goto_hor(left: bool, count: i32) {
    if let Some(win) = win_horz_neighbor(&curtab_mut(), &curwin_mut(), left, count) {
        win_goto(&win);
    }
}

pub fn win_enter(wp: &WinInfoInfo, undo_sync: bool) {
    win_enter_ext(
        wp,
        (if undo_sync { WEE_UNDO_SYNC } else { 0 })
            | WEE_TRIGGER_ENTER_AUTOCMDS
            | WEE_TRIGGER_LEAVE_AUTOCMDS,
    );
}

fn win_enter_ext(wp_in: &WinInfoInfo, flags: i32) {
    let wp = crate::globals::WinPtr::from(wp_in);
    let mut other_buffer = false;
    let curwin_invalid = (flags & WEE_CURWIN_INVALID) != 0;

    if wp == curwin_ptr() && !curwin_invalid {
        return;
    }

    if !curwin_invalid {
        leaving_window(&curwin_mut());
    }

    if !curwin_invalid && (flags & WEE_TRIGGER_LEAVE_AUTOCMDS) != 0 {
        if !ptr::eq(
            wp.borrow_mut().w_buffer.get().as_ptr(),
            g_st().curbuf.get().as_ptr(),
        ) {
            apply_autocmds(
                EVENT_BUFLEAVE,
                None,
                None,
                false,
                curbuf_opt_mut().as_deref(),
            );
            other_buffer = true;
            if !win_valid(wp.opt()) {
                return;
            }
        }
        apply_autocmds(
            EVENT_WINLEAVE,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
        if !win_valid(wp.opt()) {
            return;
        }
        if aborting() {
            return;
        }
    }

    let wp = wp.borrow_mut();
    if (flags & WEE_UNDO_SYNC) != 0 && g_st().curbuf.get() != wp.w_buffer.get() {
        u_sync(false);
    }

    if opt_vars().p_spk.first_byte() == b'c' && !curwin_invalid {
        update_topline(&curwin_mut());
    }

    if g_st().curbuf.get() != wp.w_buffer.get() {
        buf_copy_options(&(*wp).buf_mut(), BCO_ENTER | BCO_NOHELP);
    }
    if !curwin_invalid {
        g_st().prevwin.set(curwin_ptr());
        curwin_mut().w_redr_status.set(true);
    }
    set_curwin(wp);
    g_st().curbuf.set(wp.w_buffer.get());

    check_cursor(&curwin_mut());
    if !virtual_active(&curwin_mut()) {
        curwin_mut().w_cursor.set_coladd(0);
    }
    if opt_vars().p_spk.first_byte() == b'c' {
        changed_line_abv_curs();
    } else {
        win_fix_cursor((get_real_state() & (MODE_NORMAL | MODE_CMDLINE | MODE_TERMINAL)) != 0);
    }

    update_cwd(K_CD_CAUSE_WINDOW);

    entering_window(&curwin_mut());
    if (flags & WEE_TRIGGER_NEW_AUTOCMDS) != 0 {
        apply_autocmds(EVENT_WINNEW, None, None, false, curbuf_opt_mut().as_deref());
    }
    if (flags & WEE_TRIGGER_ENTER_AUTOCMDS) != 0 {
        apply_autocmds(
            EVENT_WINENTER,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
        if other_buffer {
            apply_autocmds(
                EVENT_BUFENTER,
                None,
                None,
                false,
                curbuf_opt_mut().as_deref(),
            );
        }
    }

    maketitle();
    curwin_mut().w_redr_status.set(true);
    g_st().redraw_tabline.set(true);
    if g_st().restart_edit.get() != 0 {
        redraw_later(Some(&crate::globals::curwin_mut()), UPD_VALID);
    }

    if curwin_mut().w_hl_attr_normal.get() != curwin_mut().w_hl_attr_normalnc.get() {
        redraw_later(Some(&crate::globals::curwin_mut()), UPD_NOT_VALID);
    }
    if !g_st().prevwin.get().is_null()
        && crate::globals::prevwin_mut().w_hl_attr_normal.get()
            != crate::globals::prevwin_mut().w_hl_attr_normalnc.get()
    {
        redraw_later(prevwin_opt().as_deref(), UPD_NOT_VALID);
    }

    if OptInt::from(curwin_mut().w_height.get()) < opt_vars().p_wh.get()
        && !curwin_mut().w_onebuf_opt.wo_wfh.get()
        && !curwin_mut().w_floating.get()
    {
        win_setheight(opt_vars().p_wh.get() as i32);
    } else if curwin_mut().w_height.get() == 0 {
        win_setheight(1);
    }

    if OptInt::from(curwin_mut().w_width.get()) < opt_vars().p_wiw.get()
        && !curwin_mut().w_onebuf_opt.wo_wfw.get()
        && !curwin_mut().w_floating.get()
    {
        win_setwidth(opt_vars().p_wiw.get() as i32);
    }

    if !curwin_invalid
        && win_valid(g_st().prevwin.get().opt())
        && crate::globals::prevwin_mut().w_onebuf_opt.wo_cole.get() > 0
        && !conceal_cursor_line(&crate::globals::prevwin_mut())
    {
        conceal_redraw_cursor_line(&crate::globals::prevwin_mut());
    }
    if curwin_mut().w_onebuf_opt.wo_cole.get() > 0 && !conceal_cursor_line(&curwin_mut()) {
        conceal_redraw_cursor_line(&curwin_mut());
    }

    setmouse();
    do_autochdir();
}

pub fn update_cwd(cause: CdCause) {
    let new_dir: Option<std::ffi::CString>;
    let scope: CdScope;

    if let Some(ref d) = curbuf_mut().b_localdir.get() {
        new_dir = Some(d.clone());
        scope = K_CD_SCOPE_BUFFER;
    } else if let Some(ref d) = curwin_mut().w_localdir.get() {
        new_dir = Some(d.clone());
        scope = K_CD_SCOPE_WINDOW;
    } else if let Some(d) = curtab_mut().tp_localdir.get() {
        new_dir = Some(d);
        scope = K_CD_SCOPE_TABPAGE;
    } else {
        new_dir = None;
        scope = K_CD_SCOPE_GLOBAL;
    }

    let mut cwd = [0u8; MAXPATHL];
    if !os_dirname(&mut cwd) {
        cwd[0] = 0;
    }

    if let Some(ref new_dir_cstr) = new_dir {
        if g_st().globaldir.get().is_none() && cwd[0] != 0 {
            g_st()
                .globaldir
                .set(Some(crate::memory::cstring_until_nul(&cwd)));
        }
        let dir_differs = !path_equal(
            new_dir_cstr,
            crate::memory::bytes_as_cstr(&cwd),
            K_PATH_CMP_LITERAL,
        );
        if !opt_vars().p_acd.get() && dir_differs {
            do_autocmd_dirchanged(new_dir_cstr, scope, cause, true);
        }
        if os_chdir(new_dir_cstr) == 0 && !opt_vars().p_acd.get() && dir_differs {
            do_autocmd_dirchanged(new_dir_cstr, scope, cause, false);
        }
        g_st().last_chdir_reason.set(None);
        shorten_fnames(true);
    } else if let Some(gdir) = g_st().globaldir.take() {
        let dir_differs = !path_equal(
            &gdir,
            crate::memory::bytes_as_cstr(&cwd),
            K_PATH_CMP_LITERAL,
        );
        if !opt_vars().p_acd.get() && dir_differs {
            do_autocmd_dirchanged(&gdir, K_CD_SCOPE_GLOBAL, cause, true);
        }
        if os_chdir(&gdir) == 0 && !opt_vars().p_acd.get() && dir_differs {
            do_autocmd_dirchanged(&gdir, K_CD_SCOPE_GLOBAL, cause, false);
        }
        g_st().last_chdir_reason.set(None);
        shorten_fnames(true);
    }
}

pub fn buf_jump_open_win(buf_in: &FileBuf) -> Option<crate::globals::WinPtr> {
    let buf = crate::globals::BufPtr::from(buf_in);
    if curwin_mut().w_buffer.get() == buf {
        win_enter(&curwin_mut(), false);
        let cw = curwin_ptr();
        return (!cw.is_null()).then_some(cw);
    }
    for wp in for_all_windows_ptr() {
        if wp.w_buffer.get() == buf {
            let wp_ptr = wp;
            win_enter(wp_ptr.borrow_mut(), false);
            return (!wp_ptr.is_null()).then_some(wp_ptr);
        }
    }
    None
}

pub fn buf_jump_open_tab(buf_in: &FileBuf) -> Option<crate::globals::WinPtr> {
    let buf = crate::globals::BufPtr::from(buf_in);
    if let Some(wp) = buf_jump_open_win(buf.borrow_mut()) {
        return Some(wp);
    }

    for tp in for_all_tabpages_ptr() {
        if g_st().curtab.get() == tp {
            continue;
        }
        for wp2 in win_iter_ptr(tp.tp_firstwin.get()) {
            if wp2.w_buffer.get() == buf {
                let wp2_ptr = wp2;
                goto_tabpage_win(&tp, wp2_ptr.borrow_mut());
                return if curwin_ptr() == wp2_ptr {
                    (!wp2_ptr.is_null()).then_some(wp2_ptr)
                } else {
                    None
                };
            }
        }
    }

    None
}

pub fn win_alloc(after_in: Option<&WinInfoInfo>, hidden: bool) -> crate::globals::WinPtr {
    let after = crate::globals::WinPtr::from(after_in);
    WIN_STATE.with(|s| s.last_win_id.set(s.last_win_id.get() + (1)));
    let handle = WIN_STATE.with(|s| s.last_win_id.get());
    let w_winvar = crate::eval::typval::tv_dict_item_alloc_static();
    init_var_dict(&tv_dict_alloc(), &w_winvar, VAR_SCOPE);

    let new_wp_rc = std::rc::Rc::new_cyclic(|weak| {
        let mut wp_val = WinInfoInfo::new();
        wp_val.self_weak = weak.clone();
        wp_val.handle = handle;
        wp_val.w_winvar = w_winvar;
        wp_val
    });
    let new_wp = crate::globals::WinPtr::from(new_wp_rc);

    g_st()
        .window_handles
        .borrow_mut()
        .insert(new_wp.handle, new_wp.clone());

    new_wp
        .w_grid_alloc
        .target_kind
        .set(GridTarget::Win(new_wp.handle));
    grid_assign_handle(&new_wp.w_grid_alloc);

    block_autocmds();
    if !hidden {
        let mut tp = crate::globals::TabpagePtr::NULL;
        if let Some(after_ref) = after.opt() {
            tp = crate::globals::TabpagePtr::from(win_find_tabpage(after_ref));
            if g_st().curtab.get() == tp {
                tp = crate::globals::TabpagePtr::NULL;
            }
        }
        win_append(after.opt_mut(), &new_wp, tp.opt());
    }

    new_wp.w_width.set(g_st().columns_global.get());

    new_wp.w_ns_hl.set(-1);

    new_wp.w_onebuf_opt.wo_so.set(-1);
    new_wp.w_allbuf_opt.wo_so.set(-1);
    new_wp.w_onebuf_opt.wo_sop.set(-1);
    new_wp.w_allbuf_opt.wo_sop.set(-1);
    new_wp.w_onebuf_opt.wo_siso.set(-1);
    new_wp.w_allbuf_opt.wo_siso.set(-1);

    new_wp.w_fraction.set(0);
    new_wp.w_prev_fraction_row.set(-1);

    fold_init_win(&new_wp);
    unblock_autocmds();
    new_wp.w_next_match_id.set(1000);
    new_wp
}

pub fn win_free(wp_in: &WinInfoInfo, tp: Option<&TabPageInfo>) {
    let wp = crate::globals::WinPtr::from(wp_in);
    {
        let wp = wp.borrow_mut();
        g_st().window_handles.borrow_mut().remove(&wp.handle);
        clear_folding(wp);

        wp.w_alist.set(None);

        block_autocmds();

        wp.w_ns_set.borrow_mut().clear();

        clear_winopt(&wp.w_onebuf_opt);
        clear_winopt(&wp.w_allbuf_opt);

        wp.w_p_lcs_chars.borrow_mut().multispace = Vec::new();
        wp.w_p_lcs_chars.borrow_mut().leadmultispace = Vec::new();

        vars_clear(&(*wp).w_vars().dv_hashtab);
        unref_var_dict(&(*wp).w_vars());

        if g_st().prevwin.get() == wp {
            g_st().prevwin.set(crate::globals::WinPtr::NULL);
        }
        for ttp in for_all_tabpages_mut() {
            if ttp.tp_prevwin.get() == wp {
                ttp.tp_prevwin.set(crate::globals::WinPtr::NULL);
            }
        }

        wp.w_lines.borrow_mut().clear();

        for i in 0..wp.w_tagstacklen.get() as usize {
            tagstack_clear_entry(&mut wp.w_tagstack.borrow_mut()[i]);
        }
        wp.w_tagstacklen.set(0);

        wp.w_localdir.set(None);
        wp.w_prevdir.set(None);

        stl_clear_click_defs(&mut wp.w_status_click_defs.borrow_mut());
        stl_clear_click_defs(&mut wp.w_winbar_click_defs.borrow_mut());

        for (_, lnum_click_defs) in std::mem::take(&mut *wp.w_statuscol_click_defs.borrow_mut()) {
            for (_, mut row_click_defs) in lnum_click_defs {
                stl_clear_click_defs(&mut row_click_defs);
            }
        }

        for buf in for_all_buffers_mut() {
            let mut pos_wip = None;
            let mut pos_null = None;
            for (i, wip) in buf.b_wininfo.borrow().iter().enumerate() {
                if wip.wi_win == wp {
                    pos_wip = Some(i);
                } else if wip.wi_win.is_null() {
                    pos_null = Some(i);
                }
            }

            if let Some(pw) = pos_wip {
                let mut wininfo = buf.b_wininfo.borrow_mut();
                let wip_wp = &mut wininfo[pw];
                wip_wp.wi_win = crate::globals::WinPtr::NULL;
                if wp.w_config.borrow().style == K_WIN_STYLE_MINIMAL && wip_wp.wi_optset {
                    clear_winopt(&wip_wp.wi_opt);
                    wip_wp.wi_folds.clear();
                    wip_wp.wi_optset = false;
                }
                if let Some(pn) = pos_null {
                    let pos_delete = pn.max(pw);
                    wininfo.remove(pos_delete);
                }
            }
        }

        wp.w_config.borrow_mut().title_chunks.clear();
        wp.w_config.borrow_mut().footer_chunks.clear();

        clear_matches(wp);
        free_jumplist(wp);
        qf_free_all(Some(wp));

        wp.w_p_cc_cols.borrow_mut().clear();

        win_free_grid(wp, false);

        if win_valid_any_tab(Some(wp)) {
            win_remove(wp, tp);
        }
        wp.w_buffer.set(crate::globals::BufPtr::NULL);
        wp.w_prev.set(crate::globals::WinPtr::NULL);
        wp.w_next.set(crate::globals::WinPtr::NULL);
        wp.w_frame.set(crate::globals::FramePtr::NULL);
    }
    if g_st().autocmd_busy.get() {
        g_st().au_pending_free_win.borrow_mut().push(wp);
    }

    unblock_autocmds();
}

impl Drop for WinInfoInfo {
    fn drop(&mut self) {
        for i in 0..self.w_tagstacklen.get() as usize {
            tagstack_clear_entry(&mut self.w_tagstack.borrow_mut()[i]);
        }
        self.w_tagstacklen.set(0);

        self.w_localdir.set(None);
        self.w_prevdir.set(None);

        free_jumplist(self);
        if let Some(w_winvar) = self.w_winvar.opt() {
            crate::eval::typval::tv_dict_item_free_slot(w_winvar);
        }
    }
}

pub fn win_free_grid(wp: &WinInfoInfo, reinit: bool) {
    if wp.w_grid_alloc.handle.get() != 0 && ui_has(K_UIMULTIGRID) {
        ui_call_grid_destroy(Integer::from(wp.w_grid_alloc.handle.get()));
    }
    grid_free(&wp.w_grid_alloc);
    if reinit {
        wp.w_grid_alloc.reset(GridTarget::Win(wp.handle));
    }
}

pub fn win_append(
    after_in: Option<&WinInfoInfo>,
    wp_in: &WinInfoInfo,
    tp_in: Option<&TabPageInfo>,
) {
    let after = crate::globals::WinPtr::from(after_in);
    let wp = crate::globals::WinPtr::from(wp_in);
    let tp = crate::globals::TabpagePtr::from(tp_in);
    debug_assert!(tp.is_null() || g_st().curtab.get() != tp);

    let first = if tp.is_null() {
        g_st().firstwin.get()
    } else {
        tp.tp_firstwin.get()
    };

    let before = if after.is_null() {
        first
    } else {
        after.w_next.get()
    };

    wp.borrow_mut().w_next.set(before.clone());
    wp.borrow_mut().w_prev.set(after.clone());

    if after.is_null() {
        if tp.is_null() {
            g_st().firstwin.set(wp.clone());
        } else {
            tp.tp_firstwin.set(wp.clone());
        }
    } else {
        after.borrow_mut().w_next.set(wp.clone());
    }
    if before.is_null() {
        if tp.is_null() {
            g_st().lastwin.set(wp);
        } else {
            tp.tp_lastwin.set(wp);
        }
    } else {
        before.w_prev.set(wp);
    }
}

pub fn win_remove(wp: &WinInfoInfo, tp_opt: Option<&TabPageInfo>) {
    debug_assert!(tp_opt.is_none_or(|t| g_st().curtab.get() != t));

    if let Some(prev) = wp.prev_opt() {
        prev.w_next.set(wp.w_next.get());
    } else if let Some(tp_ref) = tp_opt {
        tp_ref.tp_firstwin.set(wp.w_next.get());
    } else {
        g_st().firstwin.set(wp.w_next.get());
        curtab_mut().tp_firstwin.set(wp.w_next.get());
    }
    if let Some(next) = wp.next_opt() {
        next.w_prev.set(wp.w_prev.get());
    } else if let Some(tp_ref) = tp_opt {
        tp_ref.tp_lastwin.set(wp.w_prev.get());
    } else {
        g_st().lastwin.set(wp.w_prev.get());
        curtab_mut().tp_lastwin.set(wp.w_prev.get());
    }
}

fn frame_append(after: &Frame, frp: &Frame) {
    let frp_ptr = crate::globals::FramePtr::from(frp);
    frp.fr_next.set(after.fr_next.get());
    after.fr_next.set(frp_ptr.clone());
    if let Some(next) = frp.next_opt() {
        next.fr_prev.set(frp_ptr);
    }
    frp.fr_prev.set(crate::globals::FramePtr::from(after));
}

fn frame_insert(before: &Frame, frp: &Frame) {
    let frp_ptr = crate::globals::FramePtr::from(frp);
    frp.fr_next.set(crate::globals::FramePtr::from(before));
    frp.fr_prev.set(before.fr_prev.get());
    before.fr_prev.set(frp_ptr.clone());
    if let Some(prev) = frp.prev_opt() {
        prev.fr_next.set(frp_ptr);
    } else {
        frp.parent_mut().fr_child.set(frp_ptr);
    }
}

fn frame_remove(frp: &Frame) {
    if let Some(prev) = frp.prev_opt() {
        prev.fr_next.set(frp.fr_next.get());
    } else {
        frp.parent_mut().fr_child.set(frp.fr_next.get());
    }
    if let Some(next) = frp.next_opt() {
        next.fr_prev.set(frp.fr_prev.get());
    }
}

pub fn win_new_screensize() {
    if WIN_STATE.with(|s| s.old_rows.get()) != g_st().rows_global.get() {
        if opt_vars().p_window.get() == OptInt::from(WIN_STATE.with(|s| s.old_rows.get()) - 1)
            || (WIN_STATE.with(|s| s.old_rows.get()) == 0 && !option_was_set(K_OPT_WINDOW))
        {
            opt_vars()
                .p_window
                .set(OptInt::from(g_st().rows_global.get() - 1));
        }
        WIN_STATE.with(|s| s.old_rows.set(g_st().rows_global.get()));
        win_new_screen_rows();
    }
    if WIN_STATE.with(|s| s.old_columns.get()) != g_st().columns_global.get() {
        WIN_STATE.with(|s| s.old_columns.set(g_st().columns_global.get()));
        win_new_screen_cols();
    }
}

pub fn win_new_screen_rows() {
    if g_st().firstwin.get().is_null() {
        return;
    }
    let h = (rows_avail() as i32).max(frame_minheight(
        &topframe_mut(),
        crate::globals::WinPtr::NULL,
    ));

    frame_new_height(&topframe_mut(), h, false, true, false);
    if !frame_check_height(&topframe_mut(), h) {
        frame_new_height(&topframe_mut(), h, false, false, false);
    }

    win_comp_pos();
    win_reconfig_floats();
    compute_cmdrow();
    curtab_mut().tp_ch_used.set(opt_vars().p_ch.get());

    if !g_st().skip_win_fix_scroll.get() {
        win_fix_scroll(true);
    }
}

pub fn win_new_screen_cols() {
    if g_st().firstwin.get().is_null() {
        return;
    }

    frame_new_width(&topframe_mut(), g_st().columns_global.get(), false, true);
    if !frame_check_width(&topframe_mut(), g_st().columns_global.get()) {
        frame_new_width(&topframe_mut(), g_st().columns_global.get(), false, false);
    }

    win_comp_pos();
    win_reconfig_floats();
}

pub fn snapshot_windows_scroll_size() {
    for wp in for_all_windows_mut() {
        wp.w_last_topline.set(wp.w_topline.get());
        wp.w_last_topfill.set(wp.w_topfill.get());
        wp.w_last_leftcol.set(wp.w_leftcol.get());
        wp.w_last_skipcol.set(wp.w_skipcol.get());
        wp.w_last_width.set(wp.w_width.get());
        wp.w_last_height.set(wp.w_height.get());
    }
}

pub fn may_make_initial_scroll_size_snapshot() {
    if !WIN_STATE.with(|s| s.did_initial_scroll_size_snapshot.get()) {
        WIN_STATE.with(|s| s.did_initial_scroll_size_snapshot.set(true));
        snapshot_windows_scroll_size();
    }
}

fn make_win_info_dict(
    width: i32,
    height: i32,
    topline: i32,
    topfill: i32,
    leftcol: i32,
    skipcol: i32,
) -> Option<crate::eval::typval::DictId> {
    let d = tv_dict_alloc();
    d.dv_refcount.set(1);

    let entries: [(&CStr, i32); 6] = [
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
        if !tv_dict_add_tv(&d, key.to_bytes(), &mut tv) {
            tv_dict_unref(Some(&d));
            return None;
        }
    }
    Some(d)
}

fn check_window_scroll_resize(
    mut size_count: Option<&mut i32>,
    first_scroll_win: &mut Option<crate::globals::WinPtr>,
    first_size_win: &mut Option<crate::globals::WinPtr>,
    winlist: Option<&TvList>,
    v_event: Option<&TvDict>,
) {
    let mut tot_width: i32 = 0;
    let mut tot_height: i32 = 0;
    let mut tot_topline: i32 = 0;
    let mut tot_topfill: i32 = 0;
    let mut tot_leftcol: i32 = 0;
    let mut tot_skipcol: i32 = 0;

    for wp in for_all_windows_ptr() {
        if wp.w_floating.get() && wp.w_last_topline.get() == 0 {
            wp.w_last_topline.set(wp.w_topline.get());
            wp.w_last_topfill.set(wp.w_topfill.get());
            wp.w_last_leftcol.set(wp.w_leftcol.get());
            wp.w_last_skipcol.set(wp.w_skipcol.get());
            wp.w_last_width.set(wp.w_width.get());
            wp.w_last_height.set(wp.w_height.get());
            continue;
        }

        let eiw = wp.w_onebuf_opt.wo_eiw.as_cstr();
        let ignore_scroll = event_ignored(EVENT_WINSCROLLED, &eiw);
        let size_changed = !event_ignored(EVENT_WINRESIZED, &eiw)
            && (wp.w_last_width.get() != wp.w_width.get()
                || wp.w_last_height.get() != wp.w_height.get());
        if size_changed {
            if let Some(wl) = winlist {
                let tv = TypVal {
                    v_type: VarType::VarNumber,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(i64::from(wp.handle)),
                };
                tv_list_append_owned_tv(wl, tv);
            } else if let Some(ref mut size_count) = size_count {
                **size_count += 1;
                if first_size_win.is_none() {
                    *first_size_win = Some(wp.clone());
                }
                if first_scroll_win.is_none() && !ignore_scroll {
                    *first_scroll_win = Some(wp.clone());
                }
            }
        }

        let scroll_changed = !ignore_scroll
            && (wp.w_last_topline.get() != wp.w_topline.get()
                || wp.w_last_topfill.get() != wp.w_topfill.get()
                || wp.w_last_leftcol.get() != wp.w_leftcol.get()
                || wp.w_last_skipcol.get() != wp.w_skipcol.get());
        if scroll_changed && first_scroll_win.is_none() {
            *first_scroll_win = Some(wp.clone());
        }

        if (size_changed || scroll_changed)
            && let Some(ve) = v_event
        {
            let width = wp.w_width.get() - wp.w_last_width.get();
            let height = wp.w_height.get() - wp.w_last_height.get();
            let topline = wp.w_topline.get() - wp.w_last_topline.get();
            let topfill = wp.w_topfill.get() - wp.w_last_topfill.get();
            let leftcol = wp.w_leftcol.get() - wp.w_last_leftcol.get();
            let skipcol = wp.w_skipcol.get() - wp.w_last_skipcol.get();
            let Some(d) = make_win_info_dict(width, height, topline, topfill, leftcol, skipcol)
            else {
                break;
            };
            let mut winid = [0u8; NUMBUFLEN];
            let key_len = format_to_buf!(&mut winid[..], c"%d", wp.handle);
            let _ = key_len;
            if !tv_dict_add_dict(ve, crate::memory::bytes_as_cstr(&winid), Some(&d)) {
                tv_dict_unref(Some(&d));
                break;
            }
            d.dv_refcount.set(d.dv_refcount.get() - 1);

            tot_width += width.abs();
            tot_height += height.abs();
            tot_topline += topline.abs();
            tot_topfill += topfill.abs();
            tot_leftcol += leftcol.abs();
            tot_skipcol += skipcol.abs();
        }
    }

    if let Some(v_event) = v_event
        && let Some(alldict) = make_win_info_dict(
            tot_width,
            tot_height,
            tot_topline,
            tot_topfill,
            tot_leftcol,
            tot_skipcol,
        )
    {
        if !tv_dict_add_dict(v_event, c"all", Some(&alldict)) {
            tv_dict_unref(Some(&alldict));
        } else {
            alldict.dv_refcount.set(alldict.dv_refcount.get() - 1);
        }
    }
}

pub fn may_trigger_win_scrolled_resized() {
    let do_resize = has_event(EVENT_WINRESIZED);
    let do_scroll = has_event(EVENT_WINSCROLLED);

    if WIN_STATE.with(|s| s.win_scrolled_resized_recursive.get())
        || !(do_scroll || do_resize)
        || !WIN_STATE.with(|s| s.did_initial_scroll_size_snapshot.get())
    {
        return;
    }

    let mut size_count: i32 = 0;
    let mut first_scroll_win: Option<crate::globals::WinPtr> = None;
    let mut first_size_win: Option<crate::globals::WinPtr> = None;
    let mut dummy_scroll: Option<crate::globals::WinPtr> = None;
    let mut dummy_size: Option<crate::globals::WinPtr> = None;
    check_window_scroll_resize(
        Some(&mut size_count),
        &mut first_scroll_win,
        &mut first_size_win,
        None,
        None,
    );
    let trigger_resize = do_resize && size_count > 0;
    let trigger_scroll = do_scroll && first_scroll_win.is_some();
    if !trigger_resize && !trigger_scroll {
        return;
    }

    let mut windows_list: Option<crate::eval::typval::ListId> = None;
    if trigger_resize {
        let wl = tv_list_alloc(size_count as isize);
        check_window_scroll_resize(None, &mut dummy_scroll, &mut dummy_size, Some(&wl), None);
        windows_list = Some(wl);
    }

    let mut scroll_dict: Option<crate::eval::typval::DictId> = None;
    if trigger_scroll {
        let sd = tv_dict_alloc();
        sd.dv_refcount.set(1);
        check_window_scroll_resize(None, &mut dummy_scroll, &mut dummy_size, None, Some(&sd));
        scroll_dict = Some(sd);
    }

    snapshot_windows_scroll_size();

    WIN_STATE.with(|s| s.win_scrolled_resized_recursive.set(true));

    let mut resize_winid = [0u8; NUMBUFLEN];
    let mut resize_win = crate::globals::WinPtr::NULL;
    let mut resize_bufref: Bufref = Bufref::new();
    if trigger_resize && let Some(rw) = first_size_win {
        resize_win = rw.clone();
        format_to_buf!(&mut resize_winid[..], c"%d", rw.handle);
        set_bufref(&mut resize_bufref, rw.w_buffer.get().opt_mut());
    }

    let mut scroll_winid = [0u8; NUMBUFLEN];
    let mut scroll_win = crate::globals::WinPtr::NULL;
    let mut scroll_bufref: Bufref = Bufref::new();
    if trigger_scroll && let Some(sw) = first_scroll_win {
        scroll_win = sw.clone();
        format_to_buf!(&mut scroll_winid[..], c"%d", sw.handle);
        set_bufref(&mut scroll_bufref, sw.w_buffer.get().opt_mut());
    }

    if let Some(ref windows_list) = windows_list {
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let v_event = get_v_event(&mut save_v_event);

        if tv_dict_add_list(&v_event, c"windows", Some(windows_list)) {
            tv_dict_set_keys_readonly(&v_event);
            let buf = if bufref_valid(Some(&resize_bufref)) {
                resize_bufref.br_buf
            } else {
                g_st().curbuf.get()
            };
            let win = if win_valid_any_tab(resize_win.opt()) {
                resize_win
            } else {
                curwin_ptr()
            };
            let resize_winid_cstr = crate::memory::bytes_as_cstr(&resize_winid);
            apply_autocmds_win(
                EVENT_WINRESIZED,
                Some(resize_winid_cstr),
                Some(resize_winid_cstr),
                false,
                buf.opt_mut(),
                win.opt(),
            );
        }
        restore_v_event(&v_event, &mut save_v_event);
    }

    if let Some(ref scroll_dict) = scroll_dict {
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let v_event = get_v_event(&mut save_v_event);

        tv_dict_extend(&v_event, scroll_dict, b"move");
        tv_dict_set_keys_readonly(&v_event);
        tv_dict_unref(Some(scroll_dict));

        let buf = if bufref_valid(Some(&scroll_bufref)) {
            scroll_bufref.br_buf
        } else {
            g_st().curbuf.get()
        };
        let win = if win_valid_any_tab(scroll_win.opt()) {
            scroll_win
        } else {
            curwin_ptr()
        };
        let scroll_winid_cstr = crate::memory::bytes_as_cstr(&scroll_winid);
        apply_autocmds_win(
            EVENT_WINSCROLLED,
            Some(scroll_winid_cstr),
            Some(scroll_winid_cstr),
            false,
            buf.opt_mut(),
            win.opt(),
        );

        restore_v_event(&v_event, &mut save_v_event);
    }

    WIN_STATE.with(|s| s.win_scrolled_resized_recursive.set(false));
}

pub fn win_size_save(gap: &mut Vec<i32>) {
    gap.clear();
    gap.reserve((win_count() * 2 + 1) as usize);
    gap.push(rows_avail() as i32 + global_stl_height() - last_stl_height(false));

    for wp in for_all_windows_mut() {
        gap.push(wp.w_width.get() + wp.w_vsep_width.get());
        gap.push(wp.w_height.get());
    }
}

pub fn win_size_restore(gap: &[i32]) {
    if (win_count() * 2 + 1) as usize == gap.len()
        && gap[0] == rows_avail() as i32 + global_stl_height() - last_stl_height(false)
    {
        for _ in 0..2 {
            let mut i: usize = 1;
            for wp in for_all_windows_ptr() {
                let width = gap[i];
                let height = gap[i + 1];
                i += 2;
                if !wp.w_floating.get() {
                    frame_setwidth(&wp.frame_mut(), width, true);
                    win_setheight_win(height, &wp, true);
                }
            }
        }
        win_comp_pos();
    }
}

pub fn win_comp_pos() -> i32 {
    let mut row = tabline_height();
    let mut col: i32 = 0;

    frame_comp_pos(&topframe_mut(), &mut row, &mut col);

    for wp in for_all_windows_rev_ptr().take_while(|wp| wp.w_floating.get()) {
        if wp.w_config.borrow().relative == K_FLOAT_RELATIVE_WINDOW {
            wp.w_pos_changed.set(true);
        }
    }

    row + global_stl_height()
}

fn frame_comp_pos(topfrp: &Frame, row: &mut i32, col: &mut i32) {
    if let Some(wp) = topfrp.win_opt() {
        if wp.w_winrow.get() != *row || wp.w_wincol.get() != *col {
            wp.w_winrow.set(*row);
            wp.w_wincol.set(*col);
            redraw_later(Some(&wp), UPD_NOT_VALID);
            wp.w_redr_status.set(true);
            wp.w_pos_changed.set(true);
        }
        let h = wp.w_height.get() + wp.w_hsep_height.get() + wp.w_status_height.get();
        *row += if h > topfrp.fr_height.get() {
            topfrp.fr_height.get()
        } else {
            h
        };
        *col += wp.w_width.get() + wp.w_vsep_width.get();
    } else {
        let startrow = *row;
        let startcol = *col;
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            if topfrp.fr_layout.get() == FR_ROW {
                *row = startrow;
            } else {
                *col = startcol;
            }
            frame_comp_pos(&frp, row, col);
        }
    }
}

pub fn win_setheight(height: i32) {
    win_setheight_win(height, &crate::globals::curwin_mut(), true);
}

pub fn win_setheight_win(mut height: i32, win: &WinInfoInfo, from_top: bool) {
    let min_wmh = if win == curwin_ptr() {
        opt_vars().p_wmh.get().max(1)
    } else {
        opt_vars().p_wmh.get()
    } as i32;
    height = height.max(min_wmh + win.w_winbar_height.get());

    if win.w_floating.get() {
        win.w_config.borrow_mut().height = height.max(1);
        let cfg = win.w_config.borrow().clone();
        win_config_float(win, cfg);
        redraw_later(Some(win), UPD_VALID);
    } else {
        frame_setheight(
            &win.frame_mut(),
            height + win.w_hsep_height.get() + win.w_status_height.get(),
            from_top,
        );

        win_comp_pos();
        win_fix_scroll(true);

        redraw_all_later(UPD_NOT_VALID);
        g_st().redraw_cmdline.set(true);
    }
}

fn frame_setheight(curfrp: &Frame, mut height: i32, from_top: bool) {
    if curfrp.fr_height.get() == height {
        return;
    }

    if curfrp.fr_parent.get().is_null() {
        if height > 0 {
            frame_new_height(curfrp, height, false, false, true);
        }
    } else if curfrp.fr_parent.get().fr_layout.get() == FR_ROW {
        let h = frame_minheight(
            curfrp.fr_parent.get().borrow(),
            crate::globals::WinPtr::NULL,
        );
        height = height.max(h);

        frame_setheight(curfrp.fr_parent.get().borrow(), height, from_top);
    } else {
        let mut room: i32 = 0;
        let mut room_cmdline: i32 = 0;
        let mut room_reserved: i32 = 0;

        for run in 1..=2 {
            room = 0;
            room_reserved = 0;
            for frp in frame_iter_ptr(curfrp.fr_parent.get().fr_child.get()) {
                if !ptr::eq(frp.borrow(), curfrp)
                    && !frp.fr_win.get().is_null()
                    && frp.win_mut().w_onebuf_opt.wo_wfh.get()
                {
                    room_reserved += frp.fr_height.get();
                }
                room += frp.fr_height.get();
                if !ptr::eq(frp.borrow(), curfrp) {
                    room -= frame_minheight(&frp, crate::globals::WinPtr::NULL);
                }
            }
            if !from_top || curfrp.fr_width.get() != g_st().columns_global.get() {
                room_cmdline = 0;
            } else {
                let wp = lastwin_nofloating(None);
                room_cmdline = g_st().rows_global.get()
                    - opt_vars().p_ch.get() as i32
                    - global_stl_height()
                    - (wp.w_winrow.get()
                        + wp.w_height.get()
                        + wp.w_hsep_height.get()
                        + wp.w_status_height.get());
                room_cmdline = room_cmdline.max(0);
            }

            if height <= room + room_cmdline {
                break;
            }
            if run == 2 || curfrp.fr_width.get() == g_st().columns_global.get() {
                height = room + room_cmdline;
                break;
            }

            frame_setheight(
                curfrp.fr_parent.get().borrow(),
                height + frame_minheight(curfrp.fr_parent.get().borrow(), nowin())
                    - opt_vars().p_wmh.get() as i32
                    - 1,
                from_top,
            );
        }

        let mut take = height - curfrp.fr_height.get();

        if height > room + room_cmdline - room_reserved {
            room_reserved = room + room_cmdline - height;
        }
        if take < 0 && room - curfrp.fr_height.get() <= room_reserved {
            room_reserved = 0;
        }

        if take > 0 && room_cmdline > 0 {
            room_cmdline = room_cmdline.min(take);
            take -= room_cmdline;
            let top = crate::globals::topframe_mut();
            top.fr_height.set(top.fr_height.get() + room_cmdline);
        }

        frame_new_height(curfrp, height, false, false, true);

        for run in 0..2 {
            let forward = (run == 0) == from_top;
            let mut frp = if forward {
                curfrp.fr_next.get()
            } else {
                curfrp.fr_prev.get()
            };

            while !frp.is_null() && take != 0 {
                let h = frame_minheight(frp.borrow(), crate::globals::WinPtr::NULL);
                if room_reserved > 0
                    && !frp.fr_win.get().is_null()
                    && frp.fr_win.get().w_onebuf_opt.wo_wfh.get()
                {
                    if room_reserved >= frp.fr_height.get() {
                        room_reserved -= frp.fr_height.get();
                    } else {
                        if frp.fr_height.get() - room_reserved > take {
                            room_reserved = frp.fr_height.get() - take;
                        }

                        take -= frp.fr_height.get() - room_reserved;

                        frame_new_height(frp.borrow(), room_reserved, false, false, true);

                        room_reserved = 0;
                    }
                } else if frp.fr_height.get() - take < h {
                    take -= frp.fr_height.get() - h;

                    frame_new_height(frp.borrow(), h, false, false, true);
                } else {
                    frame_new_height(frp.borrow(), frp.fr_height.get() - take, false, false, true);

                    take = 0;
                }
                frp = if forward {
                    frp.fr_next.get()
                } else {
                    frp.fr_prev.get()
                };
            }
        }
    }
}

pub fn win_setwidth(width: i32) {
    win_setwidth_win(width, &crate::globals::curwin_mut(), true);
}

pub fn win_setwidth_win(mut width: i32, wp: &WinInfoInfo, from_left: bool) {
    if wp == curwin_ptr() {
        width = width.max(opt_vars().p_wmw.get() as i32).max(1);
    } else if width < 0 {
        width = 0;
    }
    if wp.w_floating.get() {
        wp.w_config.borrow_mut().width = width;
        let cfg = wp.w_config.borrow().clone();
        win_config_float(wp, cfg);
        redraw_later(Some(wp), UPD_NOT_VALID);
    } else {
        frame_setwidth(&wp.frame_mut(), width + wp.w_vsep_width.get(), from_left);

        win_comp_pos();
        redraw_all_later(UPD_NOT_VALID);
    }
}

fn frame_setwidth(curfrp: &Frame, mut width: i32, from_left: bool) {
    if curfrp.fr_width.get() == width {
        return;
    }

    if curfrp.fr_parent.get().is_null() {
        return;
    }

    if curfrp.fr_parent.get().fr_layout.get() == FR_COL {
        let w = frame_minwidth(
            curfrp.fr_parent.get().borrow(),
            crate::globals::WinPtr::NULL,
        );
        width = width.max(w);

        frame_setwidth(curfrp.fr_parent.get().borrow(), width, from_left);
    } else {
        let mut room: i32 = 0;
        let mut room_reserved: i32 = 0;
        for run in 1..=2 {
            room = 0;
            room_reserved = 0;
            for frp in frame_iter_ptr(curfrp.fr_parent.get().fr_child.get()) {
                if !ptr::eq(frp.borrow(), curfrp)
                    && !frp.fr_win.get().is_null()
                    && frp.win_mut().w_onebuf_opt.wo_wfw.get()
                {
                    room_reserved += frp.fr_width.get();
                }
                room += frp.fr_width.get();
                if !ptr::eq(frp.borrow(), curfrp) {
                    room -= frame_minwidth(&frp, crate::globals::WinPtr::NULL);
                }
            }

            if width <= room {
                break;
            }
            if run == 2 || OptInt::from(curfrp.fr_height.get()) >= rows_avail() {
                width = room;
                break;
            }

            frame_setwidth(
                curfrp.fr_parent.get().borrow(),
                width + frame_minwidth(curfrp.fr_parent.get().borrow(), nowin())
                    - opt_vars().p_wmw.get() as i32
                    - 1,
                from_left,
            );
        }

        let mut take = width - curfrp.fr_width.get();

        if width > room - room_reserved {
            room_reserved = room - width;
        }
        if take < 0 && room - curfrp.fr_width.get() < room_reserved {
            room_reserved = 0;
        }

        frame_new_width(curfrp, width, false, false);

        for run in 0..2 {
            let forward = (run == 0) == from_left;
            let mut frp = if forward {
                curfrp.fr_next.get()
            } else {
                curfrp.fr_prev.get()
            };

            while !frp.is_null() && take != 0 {
                let w = frame_minwidth(frp.borrow(), crate::globals::WinPtr::NULL);
                if room_reserved > 0
                    && !frp.fr_win.get().is_null()
                    && frp.fr_win.get().w_onebuf_opt.wo_wfw.get()
                {
                    if room_reserved >= frp.fr_width.get() {
                        room_reserved -= frp.fr_width.get();
                    } else {
                        if frp.fr_width.get() - room_reserved > take {
                            room_reserved = frp.fr_width.get() - take;
                        }

                        take -= frp.fr_width.get() - room_reserved;

                        frame_new_width(frp.borrow(), room_reserved, false, false);

                        room_reserved = 0;
                    }
                } else if frp.fr_width.get() - take < w {
                    take -= frp.fr_width.get() - w;

                    frame_new_width(frp.borrow(), w, false, false);
                } else {
                    frame_new_width(frp.borrow(), frp.fr_width.get() - take, false, false);

                    take = 0;
                }
                frp = if forward {
                    frp.fr_next.get()
                } else {
                    frp.fr_prev.get()
                };
            }
        }
    }
}

pub fn did_set_winminheight(_args: &mut Optset) -> Option<&'static CStr> {
    let mut first = true;
    while opt_vars().p_wmh.get() > 0 {
        let room = g_st().rows_global.get() - opt_vars().p_ch.get() as i32;
        let needed = min_rows_for_all_tabpages();
        if room >= needed {
            break;
        }
        opt_vars().p_wmh.set(opt_vars().p_wmh.get() - (1));
        if first {
            emsg(gettext(E_NOROOM));
            first = false;
        }
    }
    None
}

pub fn did_set_winminwidth(_args: &mut Optset) -> Option<&'static CStr> {
    let mut first = true;
    while opt_vars().p_wmw.get() > 0 {
        let room = g_st().columns_global.get();
        let needed = frame_minwidth(&topframe_mut(), crate::globals::WinPtr::NULL);
        if room >= needed {
            break;
        }
        opt_vars().p_wmw.set(opt_vars().p_wmw.get() - (1));
        if first {
            emsg(gettext(E_NOROOM));
            first = false;
        }
    }
    None
}

pub fn win_drag_status_line(dragwin: &WinInfoInfo, mut offset: i32) {
    let mut fr = dragwin.w_frame.get().clone();
    let mut curfr = fr.clone();
    if g_st().topframe.get() != fr {
        fr = fr.fr_parent.get();

        if fr.fr_layout.get() != FR_COL {
            curfr = fr.clone();
            if g_st().topframe.get() != fr {
                fr = fr.fr_parent.get();
            }
        }
    }

    while g_st().topframe.get() != curfr && curfr.fr_next.get().is_null() {
        if g_st().topframe.get() != fr {
            fr = fr.fr_parent.get();
        }
        curfr = fr.clone();
        if g_st().topframe.get() != fr {
            fr = fr.fr_parent.get();
        }
    }

    let mut room: i32;
    let up = offset < 0;

    if up {
        offset = -offset;
        if curfr == fr {
            room = fr.fr_height.get() - frame_minheight(fr.borrow(), crate::globals::WinPtr::NULL);
        } else {
            room = 0;

            fr = fr.fr_child.get();

            loop {
                room +=
                    fr.fr_height.get() - frame_minheight(fr.borrow(), crate::globals::WinPtr::NULL);

                if curfr == fr {
                    break;
                }

                fr = fr.fr_next.get();
            }
        }

        fr = curfr.fr_next.get();
    } else {
        room = g_st().rows_global.get() - g_st().cmdline_row.get();
        if !curfr.fr_next.get().is_null() {
            room -= opt_vars().p_ch.get() as i32 + global_stl_height();
        } else if WIN_STATE.with(|s| s.min_set_ch.get()) > 0 {
            room -= 1;
        }
        room = room.max(0);

        fr = curfr.fr_next.get();

        while !fr.is_null() {
            room += fr.fr_height.get() - frame_minheight(fr.borrow(), crate::globals::WinPtr::NULL);

            fr = fr.fr_next.get();
        }
        fr = curfr.clone();
    }

    offset = offset.min(room);
    if offset <= 0 {
        return;
    }

    if !fr.is_null() {
        frame_new_height(fr.borrow(), fr.fr_height.get() + offset, up, false, true);
    }

    if up {
        fr = curfr;
    } else {
        fr = curfr.fr_next.get();
    }
    while !fr.is_null() && offset > 0 {
        let n = frame_minheight(fr.borrow(), crate::globals::WinPtr::NULL);
        if fr.fr_height.get() - offset <= n {
            offset -= fr.fr_height.get() - n;

            frame_new_height(fr.borrow(), n, !up, false, true);
        } else {
            frame_new_height(fr.borrow(), fr.fr_height.get() - offset, !up, false, true);

            break;
        }
        if up {
            fr = fr.fr_prev.get();
        } else {
            fr = fr.fr_next.get();
        }
    }
    win_comp_pos();
    win_fix_scroll(true);

    redraw_all_later(UPD_SOME_VALID);
    showmode();
}

pub fn win_drag_vsep_line(dragwin: &WinInfoInfo, mut offset: i32) {
    let mut fr = dragwin.w_frame.get().clone();
    if g_st().topframe.get() == fr {
        return;
    }
    let mut curfr = fr.clone();

    fr = fr.fr_parent.get();

    if fr.fr_layout.get() != FR_ROW {
        if g_st().topframe.get() == fr {
            return;
        }
        curfr = fr.clone();

        fr = fr.fr_parent.get();
    }

    while curfr.fr_next.get().is_null() {
        if g_st().topframe.get() == fr {
            break;
        }
        curfr = fr.clone();

        fr = fr.fr_parent.get();

        if g_st().topframe.get() != fr {
            curfr = fr.clone();

            fr = fr.fr_parent.get();
        }
    }

    let mut room: i32;
    let left = offset < 0;

    if left {
        offset = -offset;
        room = 0;

        fr = fr.fr_child.get();

        loop {
            room += fr.fr_width.get() - frame_minwidth(fr.borrow(), crate::globals::WinPtr::NULL);

            if curfr == fr {
                break;
            }

            fr = fr.fr_next.get();
        }

        fr = curfr.fr_next.get();
    } else {
        room = 0;

        fr = curfr.fr_next.get();

        while !fr.is_null() {
            room += fr.fr_width.get() - frame_minwidth(fr.borrow(), crate::globals::WinPtr::NULL);

            fr = fr.fr_next.get();
        }
        fr = curfr.clone();
    }

    offset = offset.min(room);
    if offset <= 0 || fr.is_null() {
        return;
    }

    frame_new_width(fr.borrow(), fr.fr_width.get() + offset, left, false);

    if left {
        fr = curfr;
    } else {
        fr = curfr.fr_next.get();
    }
    while !fr.is_null() && offset > 0 {
        let n = frame_minwidth(fr.borrow(), crate::globals::WinPtr::NULL);
        if fr.fr_width.get() - offset <= n {
            offset -= fr.fr_width.get() - n;

            frame_new_width(fr.borrow(), n, !left, false);
        } else {
            frame_new_width(fr.borrow(), fr.fr_width.get() - offset, !left, false);

            break;
        }
        if left {
            fr = fr.fr_prev.get();
        } else {
            fr = fr.fr_next.get();
        }
    }
    win_comp_pos();
    redraw_all_later(UPD_NOT_VALID);
}

pub fn set_fraction(wp: &WinInfoInfo) {
    if wp.w_view_height.get() > 1 {
        wp.w_fraction
            .set((wp.w_wrow.get() * FRACTION_MULT + FRACTION_MULT / 2) / wp.w_view_height.get());
    }
}

pub fn win_fix_scroll(resize: bool) {
    if opt_vars().p_spk.first_byte() == b'c' {
        return;
    }

    g_st().skip_update_topline.set(true);
    for wp in for_all_windows_ptr() {
        if !wp.w_floating.get() && wp.w_height.get() != wp.w_prev_height.get() {
            wp.w_do_win_fix_cursor.set(true);

            if opt_vars().p_spk.first_byte() == b's'
                && wp.w_winrow.get() != wp.w_prev_winrow.get()
                && wp.w_botline.get() - 1 <= (*wp).buf_mut().b_ml.ml_line_count.get()
            {
                let diff = (wp.w_winrow.get() - wp.w_prev_winrow.get())
                    + (wp.w_height.get() - wp.w_prev_height.get());
                let cursor = wp.w_cursor.get();
                wp.w_cursor.set_lnum(wp.w_botline.get() - 1);

                if diff > 0 {
                    cursor_down_inner(&wp, diff, false);
                } else {
                    cursor_up_inner(&wp, -diff, false);
                }

                wp.w_fraction.set(FRACTION_MULT);
                scroll_to_fraction(&wp, wp.w_prev_height.get());
                wp.w_cursor.set(cursor);
                wp.w_valid.set(wp.w_valid.get() & (!VALID_WCOL));
            } else if wp == curwin_ptr() {
                wp.w_valid.set(wp.w_valid.get() & (!VALID_CROW));
            }

            invalidate_botline_win(&wp);
            validate_botline_win(&wp);
        }
        wp.w_prev_height.set(wp.w_height.get());
        wp.w_prev_winrow.set(wp.w_winrow.get());
    }
    g_st().skip_update_topline.set(false);
    if (get_real_state() & (MODE_NORMAL | MODE_CMDLINE | MODE_TERMINAL)) == 0 {
        win_fix_cursor(false);
    } else if resize {
        win_fix_cursor(true);
    }
}

fn win_fix_cursor(normal: bool) {
    let wp = curwin_mut();

    if !wp.w_do_win_fix_cursor.get()
        || (*wp).buf_mut().b_ml.ml_line_count.get() < wp.w_view_height.get()
    {
        return;
    }

    wp.w_do_win_fix_cursor.set(false);
    let so = (OptInt::from(wp.w_view_height.get() / 2).min(get_scrolloff_value(&wp))) as i32;
    let lnum = wp.w_cursor.get().lnum;

    wp.w_cursor.set_lnum(wp.w_topline.get());
    cursor_down_inner(&wp, so, false);
    let top = wp.w_cursor.get().lnum;

    wp.w_cursor.set_lnum(wp.w_botline.get() - 1);
    cursor_up_inner(&wp, so, false);
    let bot = wp.w_cursor.get().lnum;

    wp.w_cursor.set_lnum(lnum);
    let mut nlnum: Linenr = 0;
    if lnum > bot && (wp.w_botline.get() - (*wp).buf_mut().b_ml.ml_line_count.get()) != 1 {
        nlnum = bot;
    } else if lnum < top && wp.w_topline.get() != 1 {
        nlnum = if so == wp.w_view_height.get() / 2 {
            bot
        } else {
            top
        };
    }

    if nlnum != 0 {
        if normal {
            setmark(b'\'' as i32);
            wp.w_cursor.set_lnum(nlnum);
        } else {
            wp.w_fraction
                .set(if nlnum == bot { FRACTION_MULT } else { 0 });
            scroll_to_fraction(&wp, wp.w_prev_height.get());
            validate_botline_win(&curwin_mut());
        }
    }
}

pub fn win_new_height(wp: &WinInfoInfo, mut height: i32) {
    height = height.max(0);
    if wp.w_height.get() == height {
        return;
    }

    wp.w_height.set(height);
    wp.w_pos_changed.set(true);
    win_set_inner_size(wp, true);
}

pub fn scroll_to_fraction(wp: &WinInfoInfo, prev_height: i32) {
    let height = wp.w_view_height.get();

    if height > 0
        && (!wp.w_onebuf_opt.wo_scb.get() || wp == curwin_ptr())
        && (height < wp.buf_mut().b_ml.ml_line_count.get() || wp.w_topline.get() > 1)
    {
        let mut lnum = wp.w_cursor.get().lnum.max(1);
        wp.w_wrow
            .set((wp.w_fraction.get() * height - 1) / FRACTION_MULT);
        let mut line_size = plines_win_col(wp, lnum, wp.w_cursor.get().col as i64) - 1;
        let mut sline = wp.w_wrow.get() - line_size;

        if sline >= 0 {
            let rows = plines_win(wp, lnum, false);
            if sline > wp.w_view_height.get() - rows {
                sline = wp.w_view_height.get() - rows;
                wp.w_wrow.set(wp.w_wrow.get() - (rows - line_size));
            }
        }

        if sline < 0 {
            wp.w_wrow.set(line_size);
            if wp.w_wrow.get() >= wp.w_view_height.get()
                && (wp.w_view_width.get() - win_col_off(wp)) > 0
            {
                let mut skipcol: Colnr = wp.w_view_width.get() - win_col_off(wp);
                wp.w_wrow.set(wp.w_wrow.get() - (1));
                while wp.w_wrow.get() >= wp.w_view_height.get() {
                    skipcol += wp.w_view_width.get() - win_col_off(wp) + win_col_off2(wp);
                    wp.w_wrow.set(wp.w_wrow.get() - (1));
                }
                wp.w_skipcol.set(skipcol);
            }
        } else if sline > 0 {
            while sline > 0 && lnum > 1 {
                has_folding(wp, lnum, Some(&mut lnum), None);
                if lnum == 1 {
                    line_size = i32::from(!decor_conceal_line(wp, lnum - 1, false));
                    sline -= 1;
                    break;
                }
                lnum -= 1;
                if lnum == wp.w_topline.get() {
                    line_size = plines_win_nofill(wp, lnum, true) + wp.w_topfill.get();
                } else {
                    line_size = plines_win(wp, lnum, true);
                }
                sline -= line_size;
            }

            if sline < 0 {
                has_folding(wp, lnum, None, Some(&mut lnum));
                lnum += 1;
                wp.w_wrow.set(wp.w_wrow.get() - (line_size + sline));
            } else if sline > 0 {
                lnum = 1;
                wp.w_wrow.set(wp.w_wrow.get() - (sline));
            }
        }
        set_topline(wp, lnum);
    }

    if wp == curwin_ptr() {
        curs_columns(wp, false);
    }
    if prev_height > 0 {
        wp.w_prev_fraction_row.set(wp.w_wrow.get());
    }

    redraw_later(Some(wp), UPD_SOME_VALID);
    invalidate_botline_win(wp);
}

pub fn win_set_inner_size(wp: &WinInfoInfo, valid_cursor: bool) {
    let mut width = wp.w_width_request.get();
    if width == 0 {
        width = wp.w_width.get();
    }

    let prev_height = wp.w_view_height.get();
    let mut height = wp.w_height_request.get();
    if height == 0 {
        height = (wp.w_height.get() - wp.w_winbar_height.get()).max(0);
    }

    if height != prev_height {
        if height > 0 && valid_cursor {
            if wp == curwin_ptr() && (opt_vars().p_spk.first_byte() == b'c' || wp.w_floating.get())
            {
                validate_cursor(&curwin_mut());
            }
            if wp.w_view_height.get() != prev_height {
                return;
            }
            if wp.w_wrow.get() != wp.w_prev_fraction_row.get() {
                set_fraction(wp);
            }
        }
        wp.w_view_height.set(height);
        win_comp_scroll(wp);

        if valid_cursor
            && !g_st().exiting.get()
            && (opt_vars().p_spk.first_byte() == b'c' || wp.w_floating.get())
        {
            if !wp.w_onebuf_opt.wo_sms.get() {
                wp.w_skipcol.set(0);
            }
            scroll_to_fraction(wp, prev_height);
        }
        redraw_later(Some(wp), UPD_SOME_VALID);
    }

    if width != wp.w_view_width.get() {
        wp.w_view_width.set(width);
        wp.w_lines_valid.set(0);
        if valid_cursor {
            changed_line_abv_curs_win(wp);
            invalidate_botline_win(wp);
            if wp == curwin_ptr() && (opt_vars().p_spk.first_byte() == b'c' || wp.w_floating.get())
            {
                curs_columns(wp, true);
            }
        }
        redraw_later(Some(wp), UPD_NOT_VALID);
    }

    if let Some(term) = wp.buf_mut().terminal_opt() {
        terminal_check_size(term);
    }

    let float_stl_height = if wp.w_floating.get() && wp.w_status_height.get() != 0 {
        STATUS_HEIGHT
    } else {
        0
    };
    let bh = win_border_height(wp);
    let bw = win_border_width(wp);
    wp.w_height_outer
        .set(wp.w_view_height.get() + bh + wp.w_winbar_height.get() + float_stl_height);
    wp.w_width_outer.set(wp.w_view_width.get() + bw);
    wp.w_winrow_off
        .set(wp.w_border_adj[0].get() + wp.w_winbar_height.get());
    wp.w_wincol_off.set(wp.w_border_adj[3].get());

    if ui_has(K_UIMULTIGRID) {
        ui_call_win_viewport_margins(
            Integer::from(wp.w_grid_alloc.handle.get()),
            wp.handle,
            Integer::from(wp.w_winrow_off.get()),
            Integer::from(wp.w_border_adj[2].get()),
            Integer::from(wp.w_wincol_off.get()),
            Integer::from(wp.w_border_adj[1].get()),
        );
    }

    wp.w_redr_status.set(true);

    if g_st().updating_screen.get() {
        win_grid_alloc(wp);
    }
}

pub fn win_new_width(wp: &WinInfoInfo, width: i32) {
    wp.w_width.set(if width < 0 { 0 } else { width });
    wp.w_pos_changed.set(true);
    win_set_inner_size(wp, true);
}

pub fn win_default_scroll(wp: &WinInfoInfo) -> OptInt {
    OptInt::from(wp.w_view_height.get() / 2).max(1)
}

pub fn win_comp_scroll(wp: &WinInfoInfo) {
    let old_w_p_scr = wp.w_onebuf_opt.wo_scr.get();
    let new_scr = win_default_scroll(wp);
    wp.w_onebuf_opt.wo_scr.set(new_scr);

    if wp.w_onebuf_opt.wo_scr.get() != old_w_p_scr {
        let mut sctx = wp.w_onebuf_opt.wo_script_ctx[K_WIN_OPT_SCROLL].get();
        sctx.sc_sid = SID_WINLAYOUT;
        sctx.sc_lnum = 0;
        wp.w_onebuf_opt.wo_script_ctx[K_WIN_OPT_SCROLL].set(sctx);
    }
}

pub fn command_height() {
    let mut old_p_ch = curtab_mut().tp_ch_used.get() as i32;

    let mut frp = lastwin_nofloating(None).w_frame.get().clone();
    while frp.fr_width.get() != g_st().columns_global.get() && !frp.fr_parent.get().is_null() {
        frp = frp.fr_parent.get();
    }

    while !frp.fr_prev.get().is_null()
        && frp.fr_layout.get() == FR_LEAF
        && frp.fr_win.get().w_onebuf_opt.wo_wfh.get()
    {
        frp = frp.fr_prev.get();
    }

    while opt_vars().p_ch.get() > OptInt::from(old_p_ch)
        && WIN_STATE.with(|s| s.command_frame_height.get())
    {
        if frp.is_null() {
            emsg(gettext(E_NOROOM));
            opt_vars().p_ch.set(OptInt::from(old_p_ch));
            break;
        }
        let h = ((opt_vars().p_ch.get() - OptInt::from(old_p_ch)) as i32)
            .min(frp.fr_height.get() - frame_minheight(frp.borrow(), crate::globals::WinPtr::NULL));
        frame_add_height(frp.borrow(), -h);
        old_p_ch += h;

        frp = frp.fr_prev.get();
    }
    if opt_vars().p_ch.get() < OptInt::from(old_p_ch)
        && WIN_STATE.with(|s| s.command_frame_height.get())
        && !frp.is_null()
    {
        frame_add_height(
            frp.borrow(),
            (OptInt::from(old_p_ch) - opt_vars().p_ch.get()) as i32,
        );
    }

    win_comp_pos();
    win_fix_scroll(true);
    g_st()
        .cmdline_row
        .set(g_st().rows_global.get() - opt_vars().p_ch.get() as i32);
    g_st().redraw_cmdline.set(true);

    if g_st().msg_scrolled.get() == 0 && g_st().full_screen.get() {
        let grid = if !ui_has(K_UIMESSAGES) {
            msg_grid_validate();
            &g_st().msg_grid_adj
        } else {
            &g_st().default_gridview
        };
        grid_clear(
            grid,
            g_st().cmdline_row.get(),
            g_st().rows_global.get(),
            0,
            g_st().columns_global.get(),
            0,
        );
        g_st().msg_row.set(g_st().cmdline_row.get());
    }

    curtab_mut().tp_ch_used.set(opt_vars().p_ch.get());
    WIN_STATE.with(|s| s.min_set_ch.set(opt_vars().p_ch.get()));
}

fn frame_add_height(frp: &Frame, n: i32) {
    let mut fr = crate::globals::FramePtr::from(frp);
    let new_h = fr.fr_height.get() + n;
    frame_new_height(&fr, new_h, false, false, false);
    while let Some(parent) = fr.parent_opt() {
        parent.fr_height.set(parent.fr_height.get() + n);
        fr = parent;
    }
}

pub fn last_status(morewin: bool) {
    last_status_rec(
        &topframe_mut(),
        last_stl_height(morewin) > 0,
        global_stl_height() > 0,
    );
    win_float_anchor_laststatus();
}

pub fn win_remove_status_line(wp: &WinInfoInfo, add_hsep: bool) {
    wp.w_status_height.set(0);
    if add_hsep {
        wp.w_hsep_height.set(1);
    } else {
        let new_h = (if wp.w_floating.get() {
            wp.w_view_height.get()
        } else {
            wp.w_height.get()
        }) + STATUS_HEIGHT;
        win_new_height(wp, new_h);
    }
    comp_col();

    stl_clear_click_defs(&mut wp.w_status_click_defs.borrow_mut());
    *wp.w_status_click_defs.borrow_mut() = Vec::new();
}

fn find_horizontally_resizable_frame(fr: &Frame) -> Option<crate::globals::FramePtr> {
    let mut fp = crate::globals::FramePtr::from(fr);
    while fp.fr_height.get() <= frame_minheight(&fp, crate::globals::WinPtr::NULL) {
        if g_st().topframe.get() == fp {
            return None;
        }
        if fp.parent_mut().fr_layout.get() == FR_COL
            && let Some(prev) = fp.prev_opt()
        {
            fp = prev;
        } else {
            fp = fp.parent_mut();
        }
    }
    Some(fp)
}

fn resize_frame_for_status(fr: &Frame) -> bool {
    let wp = fr.win_mut();
    let fp = crate::globals::FramePtr::from(find_horizontally_resizable_frame(fr));

    if fp.is_null() {
        emsg(gettext(E_NOROOM));
        false
    } else if fp != fr {
        let old_height = fr.fr_height.get();
        frame_setheight(fr, old_height + 1, false);
        if fr.fr_height.get() == old_height {
            emsg(gettext(E_NOROOM));
            return false;
        }
        win_comp_pos();
        true
    } else {
        win_new_height(&wp, wp.w_height.get() - 1);
        true
    }
}

fn resize_frame_for_winbar(fr: &Frame) -> bool {
    let old_height = fr.fr_height.get();
    frame_setheight(fr, old_height + 1, false);
    if fr.fr_height.get() == old_height {
        emsg(gettext(E_NOROOM));
        return false;
    }
    win_comp_pos();
    true
}

fn last_status_rec(fr: &Frame, statusline: bool, is_stl_global: bool) {
    if fr.fr_layout.get() == FR_LEAF {
        let wp = fr.win_mut();
        let is_last = is_bottom_win(&wp);

        if is_last {
            if wp.w_status_height.get() != 0 && (!statusline || is_stl_global) {
                win_remove_status_line(&wp, false);
            } else if wp.w_status_height.get() == 0 && !is_stl_global && statusline {
                wp.w_status_height.set(STATUS_HEIGHT);
                if !resize_frame_for_status(fr) {
                    return;
                }
                comp_col();
            }
            if (wp.w_height.get() - wp.w_prev_height.get()).abs() == 1 {
                wp.w_prev_height.set(wp.w_height.get());
            }
        } else if wp.w_status_height.get() != 0 && is_stl_global {
            win_remove_status_line(&wp, true);
        } else if wp.w_status_height.get() == 0 && !is_stl_global {
            wp.w_status_height.set(STATUS_HEIGHT);
            wp.w_hsep_height.set(0);
            comp_col();
        }
    } else {
        for fp in frame_iter_mut(fr.fr_child.get()) {
            last_status_rec(&fp, statusline, is_stl_global);
        }
    }
}

pub fn set_winbar_win(wp: &WinInfoInfo, make_room: bool, valid_cursor: bool) -> Option<bool> {
    let winbar_height = if wp.w_floating.get() {
        i32::from(!wp.w_onebuf_opt.wo_wbr.is_empty())
    } else {
        i32::from(!opt_vars().p_wbr.is_empty() || !wp.w_onebuf_opt.wo_wbr.is_empty())
    };

    if wp.w_winbar_height.get() != winbar_height {
        if winbar_height == 1 && wp.w_view_height.get() <= 1 {
            if wp.w_floating.get() {
                emsg(gettext(E_NOROOM));
                return None;
            } else if !make_room || !resize_frame_for_winbar(&wp.frame_mut()) {
                return Some(false);
            }
        }
        wp.w_winbar_height.set(winbar_height);
        win_set_inner_size(wp, valid_cursor);

        if winbar_height == 0 {
            stl_clear_click_defs(&mut wp.w_winbar_click_defs.borrow_mut());
            *wp.w_winbar_click_defs.borrow_mut() = Vec::new();
        }
    }

    Some(true)
}

fn set_winbar_curtab(make_room: bool, valid_cursor: bool) -> bool {
    for wp in for_all_windows_ptr() {
        if set_winbar_win(&wp, make_room, valid_cursor) == Some(false) {
            return false;
        }
    }
    true
}

pub fn set_winbar(make_room: bool) {
    set_winbar_curtab(make_room, true);
}

pub fn set_winbar_all(make_room: bool) {
    let _ = set_winbar_curtab(make_room, true);

    for tp in for_all_tabpages_ptr() {
        if g_st().curtab.get() != tp {
            let mut switchwin: CtxSwitch = CtxSwitch::new();
            if ctx_switch(
                &mut switchwin,
                tp.tp_curwin.get().opt_mut(),
                Some(&tp),
                None,
                K_CTX_NO_EVENTS | K_CTX_NO_DISPLAY,
            ) {
                let _ = set_winbar_curtab(make_room, false);
            }
            ctx_restore(&mut switchwin);
        }
    }
}

pub fn tabline_height() -> i32 {
    if ui_has(K_UITABLINE) {
        return 0;
    }
    debug_assert!(!g_st().first_tabpage.get().is_null());
    match opt_vars().p_stal.get() {
        0 => 0,
        1 => i32::from(!crate::globals::first_tabpage_mut().tp_next.get().is_null()),
        _ => 1,
    }
}

pub fn global_winbar_height() -> i32 {
    i32::from(!opt_vars().p_wbr.is_empty())
}

pub fn global_stl_height() -> i32 {
    if opt_vars().p_ls.get() == 3 {
        STATUS_HEIGHT
    } else {
        0
    }
}

pub fn last_stl_height(morewin: bool) -> i32 {
    if opt_vars().p_ls.get() > 1
        || (opt_vars().p_ls.get() == 1 && (morewin || !one_window(&firstwin_mut(), None)))
    {
        STATUS_HEIGHT
    } else {
        0
    }
}

pub fn min_rows(tp: &TabPageInfo) -> i32 {
    if g_st().firstwin.get().is_null() {
        return MIN_LINES;
    }

    let mut total = frame_minheight(&tp.topframe_mut(), crate::globals::WinPtr::NULL);
    total += tabline_height() + global_stl_height();
    if (if g_st().curtab.get() == tp {
        opt_vars().p_ch.get()
    } else {
        tp.tp_ch_used.get()
    }) > 0
    {
        total += 1;
    }
    total
}

pub fn min_rows_for_all_tabpages() -> i32 {
    if g_st().firstwin.get().is_null() {
        return MIN_LINES;
    }

    let mut total: i32 = 0;
    for tp in for_all_tabpages_ptr() {
        let mut n = frame_minheight(&tp.topframe_mut(), crate::globals::WinPtr::NULL);
        if (if g_st().curtab.get() == tp {
            opt_vars().p_ch.get()
        } else {
            tp.tp_ch_used.get()
        }) > 0
        {
            n += 1;
        }
        total = total.max(n);
    }
    total += tabline_height() + global_stl_height();
    total
}

/// Minimum number of screen-columns needed to display the current windows for all tabpages.
pub fn min_cols_for_all_tabpages() -> i32 {
    if g_st().firstwin.get().is_null() {
        return MIN_COLUMNS;
    }

    let mut total: i32 = 0;
    for tp in for_all_tabpages_ptr() {
        total = total.max(frame_minwidth(
            &tp.topframe_mut(),
            crate::globals::WinPtr::NULL,
        ));
    }
    total
}

pub fn only_one_window() -> bool {
    if !crate::globals::first_tabpage_mut().tp_next.get().is_null() {
        return false;
    }

    let mut count: i32 = 0;
    for wp in for_all_windows_ptr() {
        if !wp.w_buffer.get().is_null()
            && (!((bt_help(wp.buf_opt().as_deref())
                && !bt_help(Some(&crate::globals::curbuf_mut())))
                || wp.w_floating.get()
                || wp.w_onebuf_opt.wo_pvw.get())
                || wp == curwin_ptr())
            && !is_ctx_win(&wp)
        {
            count += 1;
        }
    }
    count <= 1
}

fn check_lnums_both(do_curwin: bool, nested: bool) {
    for tp in for_all_tabpages_ptr() {
        for wp in win_iter_ptr(if g_st().curtab.get() == tp {
            g_st().firstwin.get()
        } else {
            tp.tp_firstwin.get()
        }) {
            if (do_curwin || wp != curwin_ptr()) && g_st().curbuf.get() == wp.w_buffer.get() {
                if !nested {
                    wp.w_save_cursor.w_cursor_save.set(wp.w_cursor.get());
                    wp.w_save_cursor.w_topline_save.set(wp.w_topline.get());
                }

                let mut need_adjust =
                    wp.w_cursor.get().lnum > curbuf_mut().b_ml.ml_line_count.get();
                if need_adjust {
                    wp.w_cursor.set_lnum(curbuf_mut().b_ml.ml_line_count.get());
                }
                if need_adjust || !nested {
                    wp.w_save_cursor.w_cursor_corr.set(wp.w_cursor.get());
                }

                need_adjust = wp.w_topline.get() > curbuf_mut().b_ml.ml_line_count.get();
                if need_adjust {
                    wp.w_topline.set(curbuf_mut().b_ml.ml_line_count.get());
                }
                if need_adjust || !nested {
                    wp.w_save_cursor.w_topline_corr.set(wp.w_topline.get());
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
    for tp in for_all_tabpages_ptr() {
        for wp in win_iter_mut(if g_st().curtab.get() == tp {
            g_st().firstwin.get()
        } else {
            tp.tp_firstwin.get()
        }) {
            if g_st().curbuf.get() == wp.w_buffer.get() {
                if equalpos(wp.w_save_cursor.w_cursor_corr.get(), wp.w_cursor.get())
                    && wp.w_save_cursor.w_cursor_save.get().lnum != 0
                {
                    wp.w_cursor.set(wp.w_save_cursor.w_cursor_save.get());
                }
                if wp.w_save_cursor.w_topline_corr.get() == wp.w_topline.get()
                    && wp.w_save_cursor.w_topline_save.get() != 0
                {
                    wp.w_topline.set(wp.w_save_cursor.w_topline_save.get());
                }
                if wp.w_save_cursor.w_topline_save.get() > wp.buf_mut().b_ml.ml_line_count.get() {
                    wp.w_valid.set(wp.w_valid.get() & (!VALID_TOPLINE));
                }
            }
        }
    }
}

pub fn make_snapshot(idx: i32) {
    *curtab_mut().tp_snapshot[idx as usize].borrow_mut() = Some(make_snapshot_rec(&topframe_mut()));
}

fn make_snapshot_rec(fr: &Frame) -> Box<SnapshotFrame> {
    Box::new(SnapshotFrame {
        fr_layout: fr.fr_layout.get(),
        fr_width: fr.fr_width.get(),
        fr_height: fr.fr_height.get(),
        fr_next: fr.next_opt().map(|n| make_snapshot_rec(&n)),
        fr_child: fr.child_opt().map(|c| make_snapshot_rec(&c)),
        fr_win: if fr.fr_layout.get() == FR_LEAF && fr.fr_win.get() == curwin_ptr() {
            crate::globals::curwin_ptr()
        } else {
            crate::globals::WinPtr::NULL
        },
    })
}

fn clear_snapshot(tp: &TabPageInfo, idx: i32) {
    *tp.tp_snapshot[idx as usize].borrow_mut() = None;
}

fn get_snapshot_curwin_rec(ft: &SnapshotFrame) -> Option<crate::globals::WinPtr> {
    if let Some(ref next) = ft.fr_next
        && let Some(wp) = get_snapshot_curwin_rec(next)
    {
        return Some(wp);
    }
    if let Some(ref child) = ft.fr_child
        && let Some(wp) = get_snapshot_curwin_rec(child)
    {
        return Some(wp);
    }
    (!ft.fr_win.is_null()).then(|| ft.fr_win.clone())
}

fn get_snapshot_curwin(idx: i32) -> Option<crate::globals::WinPtr> {
    let ct = curtab_mut();
    let sn = ct.tp_snapshot[idx as usize].borrow();
    get_snapshot_curwin_rec(sn.as_ref()?)
}

pub fn restore_snapshot(idx: i32, close_curwin: i32) {
    let sn_opt = curtab_mut().tp_snapshot[idx as usize].borrow_mut().take();
    if let Some(sn) = sn_opt
        && sn.fr_width == crate::globals::topframe_mut().fr_width.get()
        && sn.fr_height == crate::globals::topframe_mut().fr_height.get()
        && check_snapshot_rec(&sn, &topframe_mut())
    {
        let wp = restore_snapshot_rec(&sn, &topframe_mut());
        win_comp_pos();
        if let Some(wp_ref) = wp
            && close_curwin != 0
        {
            win_goto(&wp_ref);
        }
        redraw_all_later(UPD_NOT_VALID);
    }
}

fn check_snapshot_rec(sn: &SnapshotFrame, fr: &Frame) -> bool {
    !(sn.fr_layout != fr.fr_layout.get()
        || sn.fr_next.is_none() != fr.fr_next.get().is_null()
        || sn.fr_child.is_none() != fr.fr_child.get().is_null()
        || (sn
            .fr_next
            .as_deref()
            .is_some_and(|next| !check_snapshot_rec(next, &fr.next_mut())))
        || (sn
            .fr_child
            .as_deref()
            .is_some_and(|child| !check_snapshot_rec(child, &fr.child_mut())))
        || (!sn.fr_win.is_null() && !win_valid(sn.fr_win.opt())))
}

fn restore_snapshot_rec(sn: &SnapshotFrame, fr: &Frame) -> Option<crate::globals::WinPtr> {
    let mut wp = crate::globals::WinPtr::NULL;

    fr.fr_height.set(sn.fr_height);
    fr.fr_width.set(sn.fr_width);
    if fr.fr_layout.get() == FR_LEAF {
        let fr_h = fr.fr_height.get();
        frame_new_height(fr, fr_h, false, false, false);
        frame_new_width(fr, fr.fr_width.get(), false, false);
        wp = sn.fr_win.clone();
    }
    if let Some(ref next) = sn.fr_next
        && let Some(wp2) = restore_snapshot_rec(next, &fr.next_mut())
    {
        wp = wp2;
    }
    if let Some(ref child) = sn.fr_child
        && let Some(wp2) = restore_snapshot_rec(child, &fr.child_mut())
    {
        wp = wp2;
    }
    (!wp.is_null()).then_some(wp)
}

fn frame_check_height(topfrp: &Frame, height: i32) -> bool {
    if topfrp.fr_height.get() != height {
        return false;
    }
    if topfrp.fr_layout.get() == FR_ROW {
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            if frp.fr_height.get() != height {
                return false;
            }
        }
    }
    true
}

fn frame_check_width(topfrp: &Frame, width: i32) -> bool {
    if topfrp.fr_width.get() != width {
        return false;
    }
    if topfrp.fr_layout.get() == FR_COL {
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            if frp.fr_width.get() != width {
                return false;
            }
        }
    }
    true
}

pub fn check_colorcolumn(cc: Option<&CStr>, wp: Option<&WinInfoInfo>) -> Option<&'static CStr> {
    if wp.is_some() && wp.unwrap().w_buffer.get().is_null() {
        return None;
    }

    let cc_owned = if cc.is_none()
        && let Some(w) = wp
    {
        Some(w.w_onebuf_opt.wo_cc.to_owned())
    } else {
        None
    };
    let mut rem: &[u8] = cc.or(cc_owned.as_deref()).map_or(&[], CStr::to_bytes);

    let tw: OptInt = if let Some(wp) = wp {
        wp.buf_mut().b_p_tw.get()
    } else {
        0
    };

    let mut count: usize = 0;
    let mut color_cols = [0_i32; 256];
    while !rem.is_empty() && count < 255 {
        let mut col: i32;
        let mut skip = false;
        if matches!(rem[0], b'-' | b'+') {
            col = if rem[0] == b'-' { -1 } else { 1 };
            rem = &rem[1..];
            if !rem.first().is_some_and(u8::is_ascii_digit) {
                return Some(E_INVARG);
            }
            col *= getdigits_int(&mut rem, true, 0);
            if tw == 0 {
                skip = true;
            } else {
                col += tw as i32;
                if col < 0 {
                    skip = true;
                }
            }
        } else if rem[0].is_ascii_digit() {
            col = getdigits_int(&mut rem, true, 0);
        } else {
            return Some(E_INVARG);
        }
        if !skip {
            color_cols[count] = col - 1;
            count += 1;
        }
        if rem.is_empty() {
            break;
        }
        let Some(after_comma) = rem.strip_prefix(b",") else {
            return Some(E_INVARG);
        };
        rem = after_comma;
        if rem.is_empty() {
            return Some(E_INVARG);
        }
    }

    if let Some(wp) = wp {
        let mut cc = wp.w_p_cc_cols.borrow_mut();
        cc.clear();
        if count > 0 {
            color_cols[..count].sort_unstable();
            cc.extend_from_slice(&color_cols[..count]);
            cc.dedup();
            cc.push(-1);
        }
    }

    None
}

pub fn win_locked(wp: &WinInfoInfo) -> i32 {
    wp.w_locked.get()
}

pub fn win_get_tabwin(id: ObjHandle, tabnr: &mut i32, winnr: &mut i32) {
    *tabnr = 0;
    *winnr = 0;

    for (tnum, tp) in (1_i32..).zip(for_all_tabpages_ptr()) {
        let mut wnum: i32 = 1;
        for wp in for_all_windows_in_tab_mut(&tp) {
            if wp.handle == id {
                if win_has_winnr(&wp, &tp) {
                    *winnr = wnum;
                    *tabnr = tnum;
                }
                return;
            }
            wnum += i32::from(win_has_winnr(&wp, &tp));
        }
    }
}

pub fn win_ui_flush(validate: bool) {
    for tp in for_all_tabpages_ptr() {
        for wp in win_iter_mut(if g_st().curtab.get() == tp {
            g_st().firstwin.get()
        } else {
            tp.tp_firstwin.get()
        }) {
            if (wp.w_pos_changed.get() || wp.w_grid_alloc.pending_comp_index_update.get())
                && !wp.w_grid_alloc.chars.borrow().is_empty()
            {
                if g_st().curtab.get() == tp {
                    ui_ext_win_position(&wp, validate);
                } else {
                    ui_call_win_hide(Integer::from(wp.w_grid_alloc.handle.get()));
                    wp.w_pos_changed.set(false);
                }
                wp.w_grid_alloc.pending_comp_index_update.set(false);
            }
            if g_st().curtab.get() == tp {
                ui_ext_win_viewport(&wp);
            }
        }
    }
    pum_pos_ui_flush(false);
    msg_ui_flush();
}

pub fn lastwin_nofloating(tp: Option<&TabPageInfo>) -> crate::globals::WinPtr {
    debug_assert!(tp.is_none_or(|t| g_st().curtab.get() != t));
    let mut res = if let Some(tp_ref) = tp {
        tp_ref.lastwin_mut()
    } else {
        lastwin_mut()
    };
    while res.w_floating.get() {
        res = res.prev_mut();
    }
    res
}

pub fn win_is_focusable(wp: &WinInfoInfo) -> bool {
    wp.w_config.borrow().focusable && !wp.w_config.borrow().hide
}

impl Frame {
    #[inline]
    pub fn parent_mut(&self) -> crate::globals::FramePtr {
        self.fr_parent.get()
    }

    #[inline]
    pub fn parent_opt(&self) -> Option<crate::globals::FramePtr> {
        let p = self.fr_parent.get();
        (!p.is_null()).then_some(p)
    }

    #[inline]
    pub fn child_mut(&self) -> crate::globals::FramePtr {
        self.fr_child.get()
    }

    #[inline]
    pub fn child_opt(&self) -> Option<crate::globals::FramePtr> {
        let c = self.fr_child.get();
        (!c.is_null()).then_some(c)
    }

    #[inline]
    pub fn next_mut(&self) -> crate::globals::FramePtr {
        self.fr_next.get()
    }

    #[inline]
    pub fn next_opt(&self) -> Option<crate::globals::FramePtr> {
        let n = self.fr_next.get();
        (!n.is_null()).then_some(n)
    }

    #[inline]
    pub fn prev_mut(&self) -> crate::globals::FramePtr {
        self.fr_prev.get()
    }

    #[inline]
    pub fn prev_opt(&self) -> Option<crate::globals::FramePtr> {
        let p = self.fr_prev.get();
        (!p.is_null()).then_some(p)
    }

    #[inline]
    pub fn win_mut(&self) -> crate::globals::WinPtr {
        self.fr_win.get()
    }

    #[inline]
    pub fn win_opt(&self) -> Option<crate::globals::WinPtr> {
        let w = self.fr_win.get();
        (!w.is_null()).then_some(w)
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
        assert!(WIN_STATE.with(|s| s.split_disallowed.get()) > 0);
        assert!(WIN_STATE.with(|s| s.close_disallowed.get()) > 0);
        window_layout_unlock();
        assert!(WIN_STATE.with(|s| s.split_disallowed.get()) == 0);
        assert!(WIN_STATE.with(|s| s.close_disallowed.get()) == 0);
    }

    #[test]
    fn test_frame_list_helpers() {
        let parent = Frame::new_rc();
        let f1 = Frame::new_rc();
        let f2 = Frame::new_rc();
        let f3 = Frame::new_rc();
        let parent_ptr = crate::globals::FramePtr::from(&parent);
        f1.fr_parent.set(parent_ptr.clone());
        f2.fr_parent.set(parent_ptr.clone());
        f3.fr_parent.set(parent_ptr);
        parent.fr_child.set(crate::globals::FramePtr::from(&f1));

        frame_append(&f1, &f2);
        assert_eq!(f1.fr_next.get(), &*f2);
        assert_eq!(f2.fr_prev.get(), &*f1);

        frame_insert(&f1, &f3);
        assert_eq!(parent.fr_child.get(), &*f3);
        assert_eq!(f3.fr_next.get(), &*f1);
        assert_eq!(f1.fr_prev.get(), &*f3);

        frame_remove(&f1);
        assert_eq!(f3.fr_next.get(), &*f2);
        assert_eq!(f2.fr_prev.get(), &*f3);

        parent.clear_links();
        f1.clear_links();
        f2.clear_links();
        f3.clear_links();
    }
}
