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
#[derive(Clone, Copy)]
pub struct PosSave {
    pub w_topline_save: i32,
    pub w_topline_corr: i32,
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
    pub fr_win: std::cell::Cell<crate::globals::WinPtr>,
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
            fr_win: std::cell::Cell::new(crate::globals::WinPtr::NULL),
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
            fr_win: std::cell::Cell::new(other.fr_win.get()),
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
    pub handle: ObjHandle,
    pub w_buffer: crate::globals::BufPtr,
    pub w_s_owned: Option<Box<crate::syntax::Synblock>>,
    pub w_ns_hl: i32,
    pub w_ns_hl_winhl: i32,
    pub w_ns_hl_active: i32,
    pub w_ns_hl_attr: Option<std::rc::Rc<crate::highlight::NSHlAttr>>,
    pub w_ns_set: std::collections::BTreeSet<u32>,
    pub w_hl_id_normal: i32,
    pub w_hl_attr_normal: i32,
    pub w_hl_attr_normalnc: i32,
    pub w_hl_needs_update: i32,
    pub w_prev: crate::globals::WinPtr,
    pub w_next: crate::globals::WinPtr,
    pub w_locked: i32,
    pub w_frame: crate::globals::FramePtr,
    pub w_cursor: CursorPos,
    pub w_curswant: i32,
    pub w_set_curswant: bool,
    pub w_cursorline: i32,
    pub w_last_cursorline: i32,
    pub w_old_visual_mode: u8,
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
    pub w_topfill: i32,
    pub w_old_topfill: i32,
    pub w_botfill: bool,
    pub w_old_botfill: bool,
    pub w_leftcol: i32,
    pub w_skipcol: i32,
    pub w_last_topline: i32,
    pub w_last_topfill: i32,
    pub w_last_leftcol: i32,
    pub w_last_skipcol: i32,
    pub w_last_width: i32,
    pub w_last_height: i32,
    pub w_winrow: i32,
    pub w_height: i32,
    pub w_prev_winrow: i32,
    pub w_prev_height: i32,
    pub w_status_height: i32,
    pub w_winbar_height: i32,
    pub w_wincol: i32,
    pub w_width: i32,
    pub w_hsep_height: i32,
    pub w_vsep_width: i32,
    pub w_save_cursor: PosSave,
    pub w_do_win_fix_cursor: bool,
    pub w_wantline: i32,
    pub w_wantcol: i32,
    pub w_winrow_off: i32,
    pub w_wincol_off: i32,
    pub w_view_height: i32,
    pub w_view_width: i32,
    pub w_height_request: i32,
    pub w_width_request: i32,
    pub w_border_adj: [i32; 4],
    pub w_height_outer: i32,
    pub w_width_outer: i32,
    pub w_valid: i32,
    pub w_valid_cursor: CursorPos,
    pub w_valid_leftcol: i32,
    pub w_valid_skipcol: i32,
    pub w_viewport_invalid: bool,
    pub w_viewport_last_topline: i32,
    pub w_viewport_last_botline: i32,
    pub w_viewport_last_topfill: i32,
    pub w_viewport_last_skipcol: i32,
    pub w_cline_height: i32,
    pub w_cline_folded: bool,
    pub w_cline_row: i32,
    pub w_virtcol: i32,
    pub w_wrow: i32,
    pub w_wcol: i32,
    pub w_wcol_conceal_off: i32,
    pub w_botline: i32,
    pub w_empty_rows: i32,
    pub w_filler_rows: i32,
    pub w_lines_valid: i32,
    pub w_lines: Vec<crate::grid::Wline>,
    pub w_folds: Vec<FoldEntry>,
    pub w_fold_manual: bool,
    pub w_foldinvalid: bool,
    pub w_nrwidth: i32,
    pub w_scwidth: i32,
    pub w_minscwidth: i32,
    pub w_maxscwidth: i32,
    pub w_redr_type: i32,
    pub w_upd_rows: i32,
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
    pub w_stl_topfill: i32,
    pub w_stl_empty: bool,
    pub w_stl_recording: i32,
    pub w_stl_state: i32,
    pub w_stl_visual_mode: i32,
    pub w_stl_visual_pos: CursorPos,
    pub w_alt_fnum: i32,
    pub w_alist: Option<std::rc::Rc<crate::arglist::Alist>>,
    pub w_arg_idx: i32,
    pub w_arg_idx_invalid: bool,
    pub w_localdir: Option<std::ffi::CString>,
    pub w_prevdir: Option<std::ffi::CString>,
    pub w_onebuf_opt: Winopt,
    pub w_allbuf_opt: Winopt,
    pub w_p_cc_cols: Vec<i32>,
    pub w_p_culopt_flags: u8,
    pub w_briopt_min: i32,
    pub w_briopt_shift: i32,
    pub w_briopt_sbr: bool,
    pub w_briopt_list: i32,
    pub w_briopt_vcol: i32,
    pub w_scbind_pos: i32,
    pub w_winvar: crate::eval::typval::DictitemId,
    pub w_pcmark: CursorPos,
    pub w_prev_pcmark: CursorPos,
    pub w_jumplist: [crate::mark::Xfmark; 100],
    pub w_jumplistlen: i32,
    pub w_jumplistidx: i32,
    pub w_changelistidx: i32,
    pub w_match_head: Vec<crate::r#match::Matchitem>,
    pub w_next_match_id: i32,
    pub w_tagstack: [crate::tag::Taggy; 20],
    pub w_tagstackidx: i32,
    pub w_tagstacklen: i32,
    pub w_grid: GridView,
    pub w_grid_alloc: ScreenGrid,
    pub w_pos_changed: bool,
    pub w_floating: bool,
    pub w_kind: i32,
    pub w_config: WinConfig,
    pub w_fraction: i32,
    pub w_prev_fraction_row: i32,
    pub w_nrwidth_line_count: i32,
    pub w_statuscol_line_count: i32,
    pub w_nrwidth_width: i32,
    pub w_llist: crate::quickfix::QfInfoId,
    pub w_llist_ref: crate::quickfix::QfInfoId,
    pub w_status_click_defs: Vec<crate::mouse::StlClickDefinition>,
    pub w_winbar_click_defs: Vec<crate::mouse::StlClickDefinition>,
    pub w_statuscol_click_defs: std::collections::BTreeMap<i32, StcClicks>,
}

impl WinInfoInfo {
    #[inline]
    pub fn frame_mut(&self) -> crate::globals::FramePtr {
        self.w_frame.clone()
    }

    #[inline]
    pub fn next_opt(&self) -> Option<&'static mut WinInfoInfo> {
        self.w_next.opt_mut()
    }

    #[inline]
    pub fn prev_mut(&self) -> &'static mut WinInfoInfo {
        self.w_prev.borrow_mut()
    }

    #[inline]
    pub fn prev_opt(&self) -> Option<&'static mut WinInfoInfo> {
        self.w_prev.opt_mut()
    }

    #[inline]
    pub fn buf_mut(&self) -> &'static mut FileBuf {
        self.w_buffer.borrow_mut()
    }

    #[inline]
    pub fn buf_opt(&self) -> Option<&'static FileBuf> {
        self.w_buffer.opt()
    }

    pub const fn new() -> Self {
        const ZERO_POS: CursorPos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        Self {
            handle: 0,
            w_buffer: crate::globals::BufPtr::NULL,
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
            w_prev: crate::globals::WinPtr::NULL,
            w_next: crate::globals::WinPtr::NULL,
            w_locked: 0,
            w_frame: crate::globals::FramePtr::NULL,
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
            w_stl_empty: false,
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
            w_winvar: crate::eval::typval::DictitemId::NULL,
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
                target: GridTarget::None,
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
            w_llist: crate::quickfix::QfInfoId::NULL,
            w_llist_ref: crate::quickfix::QfInfoId::NULL,
            w_status_click_defs: Vec::new(),
            w_winbar_click_defs: Vec::new(),
            w_statuscol_click_defs: std::collections::BTreeMap::new(),
        }
    }

    #[inline]
    pub fn w_s_ptr(&self) -> &'static mut crate::syntax::Synblock {
        let this = crate::globals::WinPtr::from(self).borrow_mut();
        if let Some(ref mut owned) = this.w_s_owned {
            owned
        } else {
            &mut this.buf_mut().b_s
        }
    }

    #[inline]
    pub fn w_vars(&self) -> &'static mut TvDict {
        self.w_winvar.di_tv.vval.v_dict().borrow_mut()
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
    pub handle: ObjHandle,
    pub tp_next: crate::globals::TabpagePtr,
    pub tp_topframe: crate::globals::FramePtr,
    pub tp_curwin: crate::globals::WinPtr,
    pub tp_prevwin: crate::globals::WinPtr,
    pub tp_firstwin: crate::globals::WinPtr,
    pub tp_lastwin: crate::globals::WinPtr,
    pub tp_old_rows_avail: i64,
    pub tp_old_columns: i64,
    pub tp_ch_used: i64,
    pub tp_did_tabclosedpre: bool,
    pub tp_diffs: Vec<crate::diff::DiffBlock>,
    pub tp_diffbuf: [crate::globals::BufPtr; 8],
    pub tp_diff_invalid: i32,
    pub tp_diff_update: i32,
    pub tp_snapshot: [Option<Box<SnapshotFrame>>; 3],
    pub tp_winvar: crate::eval::typval::DictitemId,
    pub tp_localdir: Option<std::ffi::CString>,
    pub tp_prevdir: Option<std::ffi::CString>,
}

impl TabPageInfo {
    #[inline]
    pub fn diffbuf_mut(&self, idx: usize) -> &'static mut FileBuf {
        self.tp_diffbuf[idx].borrow_mut()
    }

    #[inline]
    pub fn diffbuf_opt(&self, idx: usize) -> Option<&'static mut FileBuf> {
        self.tp_diffbuf[idx].opt_mut()
    }

    #[inline]
    pub fn next_opt(&self) -> Option<&'static mut TabPageInfo> {
        self.tp_next.opt_mut()
    }

    #[inline]
    pub fn topframe_mut(&self) -> crate::globals::FramePtr {
        self.tp_topframe.clone()
    }

    #[inline]
    pub fn firstwin_mut(&self) -> &'static mut WinInfoInfo {
        self.tp_firstwin.borrow_mut()
    }

    #[inline]
    pub fn lastwin_mut(&self) -> &'static mut WinInfoInfo {
        self.tp_lastwin.borrow_mut()
    }

    #[inline]
    pub fn curwin_mut(&self) -> &'static mut WinInfoInfo {
        self.tp_curwin.borrow_mut()
    }

    pub const fn new() -> Self {
        Self {
            handle: 0,
            tp_next: crate::globals::TabpagePtr::NULL,
            tp_topframe: crate::globals::FramePtr::NULL,
            tp_curwin: crate::globals::WinPtr::NULL,
            tp_prevwin: crate::globals::WinPtr::NULL,
            tp_firstwin: crate::globals::WinPtr::NULL,
            tp_lastwin: crate::globals::WinPtr::NULL,
            tp_old_rows_avail: 0,
            tp_old_columns: 0,
            tp_ch_used: 0,
            tp_did_tabclosedpre: false,
            tp_diffs: Vec::new(),
            tp_diffbuf: [crate::globals::BufPtr::NULL; 8],
            tp_diff_invalid: 1,
            tp_diff_update: 0,
            tp_snapshot: [None, None, None],
            tp_winvar: crate::eval::typval::DictitemId::NULL,
            tp_localdir: None,
            tp_prevdir: None,
        }
    }

    #[inline]
    pub fn tp_vars(&self) -> &'static mut TvDict {
        self.tp_winvar.di_tv.vval.v_dict().borrow_mut()
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
fn nowin() -> crate::globals::WinPtr {
    crate::globals::WinPtr::new((-1isize) as *mut WinInfoInfo)
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
    ptr::eq(g_st().firstwin.as_ptr(), g_st().lastwin.as_ptr())
}

#[inline]
pub fn reset_binding(wp: &mut WinInfoInfo) {
    wp.w_onebuf_opt.wo_scb = false;
    wp.w_onebuf_opt.wo_crb = false;
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
    if curwin_mut().w_onebuf_opt.wo_wfb {
        emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
        return false;
    }
    true
}

/// Check if the current window is allowed to move to a different buffer.
pub fn check_can_set_curbuf_forceit(forceit: bool) -> bool {
    if !forceit && curwin_mut().w_onebuf_opt.wo_wfb {
        emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
        return false;
    }
    true
}

/// Return the current window, unless in the cmdline window and "prevwin" is
/// set, then return "prevwin".
pub fn prevwin_curwin() -> &'static mut WinInfoInfo {
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
pub fn swbuf_goto_win_with_buf(buf: Option<&mut FileBuf>) -> Option<&'static mut WinInfoInfo> {
    let buf_ref = buf?;
    let buf_ptr = crate::globals::BufPtr::from(buf_ref);
    let mut wp: Option<&'static mut WinInfoInfo> = None;

    if (opt_vars().swb_flags & K_OPT_SWB_FLAG_USEOPEN) != 0 {
        wp = buf_jump_open_win(buf_ptr.borrow_mut());
    }

    if wp.is_none() && (opt_vars().swb_flags & K_OPT_SWB_FLAG_USETAB) != 0 {
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
        let oldtab = g_st().curtab;
        let oldwin = curwin_mut();
        setpcmark();

        let mut wp = crate::globals::WinPtr::NULL;
        if (opt_vars().swb_flags & (K_OPT_SWB_FLAG_USEOPEN | K_OPT_SWB_FLAG_USETAB)) != 0
            && g_st().cmdmod.cmod_tab == 0
        {
            let found = buflist_findname_exp(Some(&fname_cstr));
            wp = crate::globals::WinPtr::from(swbuf_goto_win_with_buf(found));
        }

        if wp.is_null() && win_split(0, 0) {
            reset_binding(curwin_mut());
            if !do_ecmd(
                0,
                Some(&fname_cstr),
                None,
                None,
                ECMD_LASTL,
                ECMD_HIDE,
                None,
            ) {
                let old_got_int = g_st().got_int;
                g_st().got_int = false;

                win_close(curwin_mut(), false, false);
                g_st().got_int = g_st().got_int || old_got_int;

                goto_tabpage_win(oldtab.borrow_mut(), oldwin);
            } else {
                wp = curwin_ptr();
            }
        }

        if !wp.is_null() && nchar == b'F' as i32 && lnum >= 0 {
            curwin_mut().w_cursor.lnum = lnum;
            check_cursor_lnum(curwin_mut());
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
            if bt_quickfix(Some(crate::globals::curbuf_mut())) {
                do_window_newwindow(nchar, prenum, &mut cbuf);
            } else {
                win_split(prenum, 0);
            }
        }
        c if c == CTRL_V_CONST || c == b'v' as i32 => {
            reset_visual_and_resel();
            if bt_quickfix(Some(crate::globals::curbuf_mut())) {
                do_window_newwindow(nchar, prenum, &mut cbuf);
            } else {
                win_split(prenum, WSP_VERT);
            }
        }
        c if c == CTRL_HAT_CONST || c == b'^' as i32 => {
            reset_visual_and_resel();

            if buflist_findnr(if prenum == 0 {
                curwin_mut().w_alt_fnum
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
                if wp2.w_onebuf_opt.wo_pvw {
                    wp = crate::globals::WinPtr::from(&*wp2);
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
                    let mut last_focusable = g_st().firstwin;
                    let mut cur = firstwin_mut();
                    while {
                        prenum -= 1;
                        prenum > 0
                    } {
                        if !cur.w_floating || win_is_focusable(cur) {
                            last_focusable = crate::globals::WinPtr::from(&*cur);
                        }
                        let Some(next) = cur.next_opt() else {
                            break;
                        };
                        cur = next;
                    }
                    let mut opt = Some(cur);
                    while let Some(ref w) = opt
                        && w.w_floating
                        && !win_is_focusable(w)
                    {
                        opt = w.next_opt();
                    }
                    opt.map_or(last_focusable, |w| crate::globals::WinPtr::from(&*w))
                } else if nchar == b'W' as i32 {
                    let mut opt = curwin_mut().prev_opt().or_else(|| Some(lastwin_mut()));
                    while let Some(ref w) = opt
                        && w.w_floating
                        && !win_is_focusable(w)
                    {
                        opt = w.prev_opt();
                    }
                    opt.map_or(crate::globals::WinPtr::NULL, |w| {
                        crate::globals::WinPtr::from(&*w)
                    })
                } else {
                    let mut opt = curwin_mut().next_opt();
                    while let Some(ref w) = opt
                        && w.w_floating
                        && !win_is_focusable(w)
                    {
                        opt = w.next_opt();
                    }
                    opt.map_or(g_st().firstwin, |w| crate::globals::WinPtr::from(&*w))
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
            if one_window(curwin_mut(), None) {
                msg(gettext(M_ONLYONE), 0);
            } else {
                let oldtab = g_st().curtab;
                let wp = curwin_ptr();
                if win_new_tabpage(prenum, None, true, None).is_some()
                    && valid_tabpage(oldtab.opt())
                {
                    let newtab = g_st().curtab;
                    goto_tabpage_tp(oldtab.opt_mut(), true, true);
                    if wp == curwin_ptr() {
                        win_close(curwin_mut(), false, false);
                    }
                    if valid_tabpage(newtab.opt()) {
                        goto_tabpage_tp(newtab.opt_mut(), true, true);
                        apply_autocmds(EVENT_TABNEWENTERED, None, None, false, curbuf_opt_mut());
                    }
                }
            }
        }
        c if c == b't' as i32 || c == CTRL_T_CONST => {
            win_goto(firstwin_mut());
        }
        c if c == b'b' as i32 || c == CTRL_B_CONST => {
            win_goto(lastwin_nofloating(None));
        }
        c if c == b'p' as i32 || c == CTRL_P => {
            if !win_valid(g_st().prevwin.opt()) || !win_is_focusable(crate::globals::prevwin_mut())
            {
                beep_flush();
            } else {
                win_goto(crate::globals::prevwin_mut());
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
            if one_window(curwin_mut(), None) {
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
                win_splitmove(curwin_mut(), prenum, dir);
            }
        }
        c if c == b'=' as i32 => {
            let mod_ = g_st().cmdmod.cmod_split & (WSP_VERT | WSP_HOR);
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
            win_setheight(curwin_mut().w_height + prenum1);
        }
        c if c == b'-' as i32 => {
            win_setheight(curwin_mut().w_height - prenum1);
        }
        c if c == CTRL_CONST_CONST || c == b'_' as i32 => {
            win_setheight(if prenum != 0 {
                prenum
            } else {
                g_st().rows_global - WIN_STATE.with(|s| s.min_set_ch.get()) as i32
            });
        }
        c if c == b'>' as i32 => {
            win_setwidth(curwin_mut().w_width + prenum1);
        }
        c if c == b'<' as i32 => {
            win_setwidth(curwin_mut().w_width - prenum1);
        }
        c if c == b'|' as i32 => {
            win_setwidth(if prenum != 0 {
                prenum
            } else {
                g_st().columns_global
            });
        }
        c if c == b'}' as i32 || c == b']' as i32 || c == CTRL_RSB_CONST => {
            if nchar == b'}' as i32 {
                if prenum != 0 {
                    g_st().g_do_tagpreview = prenum;
                } else {
                    g_st().g_do_tagpreview = opt_vars().p_pvh as i32;
                }
            } else {
                g_st().g_do_tagpreview = 0;
            }

            if prenum != 0 {
                g_st().postponed_split = prenum;
            } else {
                g_st().postponed_split = -1;
            }

            do_nv_ident(CTRL_RSB_CONST, NUL as i32);
            g_st().postponed_split = 0;
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
                ident,
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
            curwin_mut().w_set_curswant = true;
        }
        c if c == K_KENTER || c == CAR => {
            if bt_quickfix(Some(crate::globals::curbuf_mut())) {
                qf_view_result(true);
            }
        }
        c if c == b'g' as i32 || c == CTRL_G => {
            g_st().no_mapping += 1;
            g_st().allow_keys += 1;
            if xchar == NUL as i32 {
                xchar = plain_vgetc();
            }
            langmap_adjust(&mut xchar, true);
            g_st().no_mapping -= 1;
            g_st().allow_keys -= 1;
            add_to_showcmd(xchar);

            match xchar {
                xc if xc == b'}' as i32 || xc == b']' as i32 || xc == CTRL_RSB_CONST => {
                    if xchar == b'}' as i32 {
                        xchar = CTRL_RSB_CONST;
                        if prenum != 0 {
                            g_st().g_do_tagpreview = prenum;
                        } else {
                            g_st().g_do_tagpreview = opt_vars().p_pvh as i32;
                        }
                    }
                    if prenum != 0 {
                        g_st().postponed_split = prenum;
                    } else {
                        g_st().postponed_split = -1;
                    }
                    do_nv_ident(b'g' as i32, xchar);
                    g_st().postponed_split = 0;
                }
                xc if xc == b'f' as i32 || xc == b'F' as i32 => {
                    g_st().cmdmod.cmod_tab = tabpage_index(g_st().curtab.opt()) + 1;
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
                    if curwin_mut().w_floating || !ui_has(K_UIMULTIGRID) {
                        beep_flush();
                        return;
                    }
                    let mut config = WIN_CONFIG_INIT;
                    config.width = curwin_mut().w_width;
                    config.height = curwin_mut().w_height;
                    config.external = true;
                    let mut err = ERROR_INIT;
                    if win_new_float(Some(curwin_mut()), false, config, &mut err).is_none() {
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

pub fn win_set_buf(win: &mut WinInfoInfo, buf: &mut FileBuf, err: &mut Error) {
    let win_handle = win.handle;
    let tab = win_find_tabpage(&*win);

    g_st().redrawing_disabled += 1;

    let mut switchwin: CtxSwitch = CtxSwitch::new();

    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    let win_ok = ctx_switch(
        &mut switchwin,
        Some(crate::globals::WinPtr::from(&*win).borrow_mut()),
        tab,
        None,
        K_CTX_NO_DISPLAY | K_CTX_KEEP_CWD,
    );
    if win_ok {
        let save_acd = opt_vars().p_acd;
        if !switchwin.cs_same_win {
            opt_vars().p_acd = false;
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

/// Return the number of fold columns to display
pub fn win_fdccol_count(wp: &mut WinInfoInfo) -> i32 {
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

pub fn ui_ext_win_position(wp: &mut WinInfoInfo, validate: bool) {
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
        let mut grid_comp_row = g_st().default_grid.comp_row;
        let mut grid_comp_col = g_st().default_grid.comp_col;
        let mut grid_handle = g_st().default_grid.handle;
        let mut row = c.row;
        let mut col = c.col;
        if c.relative == K_FLOAT_RELATIVE_WINDOW {
            let mut dummy = ERROR_INIT;
            let win = find_window_by_handle(c.window, &mut dummy);
            api_clear_error(&mut dummy);
            if !win.is_null() {
                let win = win.borrow_mut();
                if win.w_pos_changed && !win.w_grid_alloc.chars.is_empty() && win_valid(Some(&*win))
                {
                    ui_ext_win_position(&mut *win, validate);
                }
                let mut row_off: i32 = 0;
                let mut col_off: i32 = 0;
                win_grid_alloc(&mut *win);
                let g = grid_adjust(&mut win.w_grid, &mut row_off, &mut col_off);
                grid_comp_row = g.comp_row;
                grid_comp_col = g.comp_col;
                grid_handle = g.handle;
                row += f64::from(row_off);
                col += f64::from(col_off);
                if c.bufpos.lnum >= 0 {
                    let lnum = (c.bufpos.lnum + 1).min(win.buf_mut().b_ml.ml_line_count);
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
                        &mut *win, &mut pos, &mut trow, &mut tcol, &mut tcolc, &mut tcole, true,
                    );
                    row += f64::from(trow - 1);
                    col += f64::from(tcol - 1);
                }
            }
        } else if c.relative == K_FLOAT_RELATIVE_LASTSTATUS {
            row += f64::from(g_st().rows_global - opt_vars().p_ch as i32 - last_stl_height(false));
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

        let mut comp_row = (row as i32) - if south { wp.w_height_outer } else { 0 };
        let mut comp_col = (col as i32) - if east { wp.w_width_outer } else { 0 };
        let above_ch = if wp.w_config.zindex < K_ZINDEX_MESSAGES {
            opt_vars().p_ch as i32
        } else {
            0
        };
        comp_row += grid_comp_row;
        comp_col += grid_comp_col;
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
            if resort {
                ui_comp_compose_grid(&mut wp.w_grid_alloc);
            }
            if ui_has(K_UIMULTIGRID) {
                let anchor = cstr_as_string(Some(FLOAT_ANCHOR_STR[c.anchor as usize]));
                ui_call_win_float_pos(
                    Integer::from(wp.w_grid_alloc.handle),
                    wp.handle,
                    anchor,
                    Integer::from(grid_handle),
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

pub fn ui_ext_win_viewport(wp: &mut WinInfoInfo) {
    if (wp == curwin_ptr() || ui_has(K_UIMULTIGRID)) && wp.w_viewport_invalid && wp.w_redr_type == 0
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
                true,
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
                true,
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
    if wp.is_some_and(|w| w.buf_mut().b_locked_split != 0) {
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

    flags |= g_st().cmdmod.cmod_split;
    if (flags & WSP_TOP) != 0 && (flags & WSP_BOT) != 0 {
        emsg(gettext(
            c"E442: Can't split topleft and botright at the same time",
        ));
        return false;
    }

    if (flags & WSP_HELP) != 0 {
        make_snapshot(SNAP_HELP_IDX as i32);
    } else {
        clear_snapshot(curtab_mut(), SNAP_HELP_IDX as i32);
    }

    if (flags & WSP_QUICKFIX) != 0 {
        make_snapshot(SNAP_QUICKFIX_IDX as i32);
    } else {
        clear_snapshot(curtab_mut(), SNAP_QUICKFIX_IDX as i32);
    }

    win_split_ins(size, flags, None, 0, None).is_some()
}

pub fn win_split_ins(
    size: i32,
    flags: i32,
    new_wp_in: Option<&mut WinInfoInfo>,
    dir: i32,
    to_flatten_in: Option<&Frame>,
) -> Option<&'static mut WinInfoInfo> {
    let new_wp = crate::globals::WinPtr::from(new_wp_in);
    let to_flatten = crate::globals::FramePtr::from(to_flatten_in);
    let mut wp = new_wp;

    if let Some(new_wp) = new_wp.opt()
        && is_ctx_win(new_wp)
    {
        return None;
    }

    if new_wp.is_null() {
        trigger_winnewpre();
    }

    let oldwin = if (flags & WSP_TOP) != 0 {
        g_st().firstwin
    } else if (flags & WSP_BOT) != 0 || curwin_mut().w_floating {
        crate::globals::WinPtr::from(lastwin_nofloating(None))
    } else {
        curwin_ptr()
    };
    let oldwin = oldwin.borrow_mut();

    let mut need_status: i32 = 0;
    let mut new_size = size;
    let vertical = (flags & WSP_VERT) != 0;
    let toplevel = (flags & (WSP_TOP | WSP_BOT)) != 0;

    if one_window(firstwin_mut(), None) && opt_vars().p_ls == 1 && oldwin.w_status_height == 0 {
        if OptInt::from(oldwin.w_height) <= opt_vars().p_wmh {
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
        let wmw1 = if opt_vars().p_wmw == 0 {
            1
        } else {
            opt_vars().p_wmw as i32
        };
        let mut needed = wmw1 + 1;
        if (flags & WSP_ROOM) != 0 {
            needed += opt_vars().p_wiw as i32 - wmw1;
        }
        let mut minwidth: i32;
        let available: i32;
        if toplevel {
            minwidth = frame_minwidth(&topframe_mut(), nowin());
            available = crate::globals::topframe_mut().fr_width.get();
            needed += minwidth;
        } else if opt_vars().p_ea {
            minwidth = frame_minwidth(oldwin.w_frame.borrow(), nowin());

            let mut prevfrp = oldwin.w_frame.clone();
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
            minwidth = frame_minwidth(oldwin.w_frame.borrow(), nowin());

            available = oldwin.frame_mut().fr_width.get();
            needed += minwidth;
        }
        if available < needed {
            emsg(gettext(E_NOROOM));
            return None;
        }
        if new_size == 0 {
            new_size = oldwin.w_width / 2;
        }
        new_size = new_size.min(available - minwidth - 1).max(wmw1);

        if OptInt::from(oldwin.w_width - new_size - 1) < opt_vars().p_wmw {
            do_equal = true;
        }

        if oldwin.w_onebuf_opt.wo_wfw {
            win_setwidth_win(oldwin.w_width + new_size + 1, &mut *oldwin, true);
        }

        if !do_equal
            && opt_vars().p_ea
            && size == 0
            && opt_vars().p_ead.first_byte() != b'v'
            && !oldwin.frame_mut().fr_parent.get().is_null()
        {
            for frp in frame_iter_mut(oldwin.frame_mut().fr_parent.get().fr_child.get()) {
                if frp.fr_win.get() != oldwin
                    && !frp.fr_win.get().is_null()
                    && (frp.win_mut().w_width > new_size
                        || frp.win_mut().w_width > (oldwin.w_width - new_size - 1))
                {
                    do_equal = true;
                    break;
                }
            }
        }
    } else {
        let wmh1 = (opt_vars().p_wmh as i32).max(1) + oldwin.w_winbar_height;
        let mut needed = wmh1 + STATUS_HEIGHT;
        if (flags & WSP_ROOM) != 0 {
            needed += opt_vars().p_wh as i32 - wmh1 + oldwin.w_winbar_height;
        }
        if opt_vars().p_ch < 1 {
            needed += 1;
        }
        let mut minheight: i32;
        let available: i32;
        if toplevel {
            minheight = frame_minheight(&topframe_mut(), nowin()) + need_status;
            available = crate::globals::topframe_mut().fr_height.get();
            needed += minheight;
        } else if opt_vars().p_ea {
            minheight = frame_minheight(oldwin.w_frame.borrow(), nowin()) + need_status;

            let mut prevfrp = oldwin.w_frame.clone();
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
            minheight = frame_minheight(oldwin.w_frame.borrow(), nowin()) + need_status;

            available = oldwin.frame_mut().fr_height.get();
            needed += minheight;
        }
        if available < needed {
            emsg(gettext(E_NOROOM));
            return None;
        }
        oldwin_height = oldwin.w_height;
        if need_status != 0 {
            oldwin.w_status_height = STATUS_HEIGHT;
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

        if oldwin.w_onebuf_opt.wo_wfh {
            set_fraction(&mut *oldwin);
            did_set_fraction = true;

            win_setheight_win(
                oldwin.w_height + new_size + STATUS_HEIGHT,
                &mut *oldwin,
                true,
            );
            oldwin_height = oldwin.w_height;
        }

        if !do_equal
            && opt_vars().p_ea
            && size == 0
            && opt_vars().p_ead.first_byte() != b'h'
            && !oldwin.frame_mut().fr_parent.get().is_null()
        {
            for frp in frame_iter_mut(oldwin.frame_mut().fr_parent.get().fr_child.get()) {
                if frp.fr_win.get() != oldwin
                    && !frp.fr_win.get().is_null()
                    && (frp.win_mut().w_height > new_size
                        || frp.win_mut().w_height > oldwin_height - new_size - STATUS_HEIGHT)
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
                    opt_vars().p_spr
                } else {
                    opt_vars().p_sb
                }))
    {
        if new_wp.is_null() {
            wp = win_alloc(Some(&mut *oldwin), false).into();
        } else {
            win_append(Some(&mut *oldwin), wp.borrow_mut(), None);
        }
    } else if new_wp.is_null() {
        wp = win_alloc(oldwin.w_prev.opt_mut(), false).into();
    } else {
        win_append(oldwin.w_prev.opt_mut(), wp.borrow_mut(), None);
    }

    if new_wp.is_null() && wp.is_null() {
        return None;
    }
    let wp = wp.borrow_mut();

    if new_wp.is_null() {
        new_frame(wp);
        win_init(&mut *wp, curwin_mut(), flags);
    } else if wp.w_floating {
        ui_comp_remove_grid(&mut wp.w_grid_alloc);
        if ui_has(K_UIMULTIGRID) {
            wp.w_pos_changed = true;
        } else {
            ui_call_win_hide(Integer::from(wp.w_grid_alloc.handle));
            win_free_grid(&mut *wp, true);
        }

        if wp.w_config.external {
            for tp in for_all_tabpages_ptr() {
                if g_st().curtab != tp && tp.tp_curwin == wp {
                    tp.tp_curwin = tp.tp_firstwin;
                }
            }
        }

        wp.w_floating = false;
        new_frame(&mut *wp);

        clear_float_config(&mut wp.w_config, true);
        wp.w_border_adj = [0; 4];
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
            curfrp = g_st().topframe.clone();
        }
        before = (flags & WSP_TOP) != 0;
    } else {
        curfrp = oldwin.w_frame.clone();
        if (flags & WSP_BELOW) != 0 {
            before = false;
        } else if (flags & WSP_ABOVE) != 0 {
            before = true;
        } else if vertical {
            before = !opt_vars().p_spr;
        } else {
            before = !opt_vars().p_sb;
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
            oldwin.w_frame = frp;
        } else {
            frp = frp.fr_child.get();

            while !frp.is_null() {
                frp.fr_parent.set(curfrp.clone());
                frp = frp.fr_next.get();
            }
        }
    }

    let frp = if new_wp.is_null() {
        wp.w_frame.clone()
    } else {
        new_wp.borrow_mut().w_frame.clone()
    };

    frp.fr_parent.set(curfrp.fr_parent.get());

    if before {
        frame_insert(curfrp.borrow(), &frp);
    } else {
        frame_append(curfrp.borrow(), &frp);
    }

    if !did_set_fraction {
        set_fraction(&mut *oldwin);
    }
    wp.w_fraction = oldwin.w_fraction;

    if vertical {
        wp.w_onebuf_opt.wo_scr = curwin_mut().w_onebuf_opt.wo_scr;

        if need_status != 0 {
            win_new_height(oldwin, oldwin.w_height - 1);
            oldwin.w_status_height = need_status;
        }
        if toplevel {
            wp.w_winrow = tabline_height();
            let ls_h = i32::from(opt_vars().p_ls == 1 || opt_vars().p_ls == 2);

            win_new_height(&mut *wp, curfrp.fr_height.get() - ls_h);

            wp.w_status_height = ls_h;
            wp.w_hsep_height = 0;
        } else {
            wp.w_winrow = oldwin.w_winrow;
            win_new_height(&mut *wp, oldwin.w_height);
            wp.w_status_height = oldwin.w_status_height;
            wp.w_hsep_height = oldwin.w_hsep_height;
        }

        frp.fr_height.set(curfrp.fr_height.get());

        win_new_width(&mut *wp, new_size);
        if before {
            wp.w_vsep_width = 1;
        } else {
            wp.w_vsep_width = oldwin.w_vsep_width;
            oldwin.w_vsep_width = 1;
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
            win_new_width(oldwin, oldwin.w_width - (new_size + 1));
        }
        if before {
            wp.w_wincol = oldwin.w_wincol;
            oldwin.w_wincol += new_size + 1;
        } else {
            wp.w_wincol = oldwin.w_wincol + oldwin.w_width + 1;
        }
        frame_fix_width(&mut *oldwin);
        frame_fix_width(&mut *wp);
    } else {
        let is_stl_global = global_stl_height() > 0;
        if toplevel {
            wp.w_wincol = 0;
            win_new_width(&mut *wp, g_st().columns_global);
            wp.w_vsep_width = 0;
        } else {
            wp.w_wincol = oldwin.w_wincol;
            win_new_width(&mut *wp, oldwin.w_width);
            wp.w_vsep_width = oldwin.w_vsep_width;
        }

        frp.fr_width.set(curfrp.fr_width.get());

        win_new_height(&mut *wp, new_size);
        let old_status_height = oldwin.w_status_height;
        if before {
            wp.w_hsep_height = i32::from(is_stl_global);
        } else {
            wp.w_hsep_height = oldwin.w_hsep_height;
            oldwin.w_hsep_height = i32::from(is_stl_global);
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
                if !((flags & WSP_BOT) != 0 && opt_vars().p_ls == 0) {
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
            win_new_height(&mut *oldwin, oldwin_height - (new_size + STATUS_HEIGHT));
        }

        if before {
            wp.w_winrow = oldwin.w_winrow;
            if is_stl_global {
                wp.w_status_height = 0;
                oldwin.w_winrow += wp.w_height + 1;
            } else {
                wp.w_status_height = STATUS_HEIGHT;
                oldwin.w_winrow += wp.w_height + STATUS_HEIGHT;
            }
        } else if is_stl_global {
            wp.w_winrow = oldwin.w_winrow + oldwin.w_height + 1;
            wp.w_status_height = 0;
        } else {
            wp.w_winrow = oldwin.w_winrow + oldwin.w_height + STATUS_HEIGHT;
            wp.w_status_height = old_status_height;
            if (flags & WSP_BOT) == 0 {
                oldwin.w_status_height = STATUS_HEIGHT;
            }
        }
        frame_fix_height(&mut *wp);
        frame_fix_height(&mut *oldwin);
    }

    if toplevel {
        win_comp_pos();
    }

    redraw_later(Some(&mut *wp), UPD_NOT_VALID);
    redraw_later(Some(&mut *oldwin), UPD_NOT_VALID);
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
            Some(&*wp),
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
    } else if !is_ctx_win(&*wp) {
        win_fix_scroll(false);
    }

    let i: i32;
    if (flags & WSP_VERT) != 0 {
        i = opt_vars().p_wiw as i32;
        if size != 0 {
            opt_vars().p_wiw = OptInt::from(size);
        }
    } else {
        i = opt_vars().p_wh as i32;
        if size != 0 {
            opt_vars().p_wh = OptInt::from(size);
        }
    }

    let mut oldwin_ptr = crate::globals::WinPtr::from(&*oldwin);
    let wp_ptr = crate::globals::WinPtr::from(&*wp);
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
        opt_vars().p_wiw = OptInt::from(i);
    } else {
        opt_vars().p_wh = OptInt::from(i);
    }

    if win_valid(oldwin_ptr.opt()) {
        oldwin_ptr.w_pos_changed = true;
    }

    wp_ptr.opt_mut()
}
pub fn win_init(newp: &mut WinInfoInfo, oldp: &mut WinInfoInfo, flags: i32) {
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
        newp.w_llist = crate::quickfix::QfInfoId::NULL;
        newp.w_llist_ref = crate::quickfix::QfInfoId::NULL;
    } else {
        copy_loclist_stack(oldp, newp);
    }
    newp.w_localdir = oldp.w_localdir.clone();
    newp.w_prevdir = oldp.w_prevdir.clone();

    if opt_vars().p_spk.first_byte() != b'c' {
        if opt_vars().p_spk.first_byte() == b't' {
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

fn win_init_some(newp: &mut WinInfoInfo, oldp: &mut WinInfoInfo) {
    newp.w_alist = oldp.w_alist.clone();
    newp.w_arg_idx = oldp.w_arg_idx;

    win_copy_options(oldp, newp);
}

pub fn win_valid(win: Option<&WinInfoInfo>) -> bool {
    tabpage_win_valid(g_st().curtab.opt(), win)
}

pub fn tabpage_win_valid(tp: Option<&TabPageInfo>, win: Option<&WinInfoInfo>) -> bool {
    let (Some(tp_ref), Some(win_ref)) = (tp, win) else {
        return false;
    };
    let win_ptr = WinPtr::from(win_ref);
    let mut wp = if g_st().curtab == tp_ref {
        g_st().firstwin
    } else {
        tp_ref.tp_firstwin
    };
    while let Some(w) = wp.opt() {
        if wp == win_ptr {
            return true;
        }
        wp = w.w_next;
    }
    false
}

#[expect(clippy::manual_find)]
pub fn win_find_by_handle(handle: ObjHandle) -> Option<&'static mut WinInfoInfo> {
    for wp in for_all_windows_ptr() {
        if wp.handle == handle {
            return Some(wp);
        }
    }
    None
}

pub fn win_valid_any_tab(win: Option<&WinInfoInfo>) -> bool {
    let mut tp = g_st().first_tabpage;
    while let Some(t) = tp.opt() {
        if tabpage_win_valid(Some(t), win) {
            return true;
        }
        tp = t.tp_next;
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
        ((OptInt::from(curwin_mut().w_width + curwin_mut().w_vsep_width)
            - (opt_vars().p_wiw - opt_vars().p_wmw)) as i32)
            / (opt_vars().p_wmw as i32 + 1)
    } else {
        ((OptInt::from(
            curwin_mut().w_height + curwin_mut().w_hsep_height + curwin_mut().w_status_height,
        ) - (opt_vars().p_wh - opt_vars().p_wmh)) as i32)
            / (opt_vars().p_wmh as i32 + STATUS_HEIGHT + global_winbar_height())
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

fn win_exchange(mut prenum: i32) {
    if curwin_mut().w_floating {
        emsg(E_FLOATEXCHANGE);
        return;
    }

    if one_window(curwin_mut(), None) {
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

    let wp2 = curwin_mut().w_prev;
    let frp2 = curwin_mut().frame_mut().fr_prev.get();
    if wp.w_prev != curwin_ptr() {
        win_remove(curwin_mut(), None);

        frame_remove(curwin_mut().w_frame.borrow());

        win_append(wp.w_prev.opt_mut(), curwin_mut(), None);

        frame_insert(frp.borrow(), curwin_mut().w_frame.borrow());
    }
    if wp2 != wp {
        win_remove(&mut *wp, None);

        frame_remove(wp.w_frame.borrow());

        win_append(wp2.opt_mut(), &mut *wp, None);
        if frp2.is_null() {
            frame_insert(
                wp.frame_mut().fr_parent.get().fr_child.get().borrow(),
                wp.w_frame.borrow(),
            );
        } else {
            frame_append(frp2.borrow(), wp.w_frame.borrow());
        }
    }
    std::mem::swap(&mut curwin_mut().w_status_height, &mut wp.w_status_height);
    std::mem::swap(&mut curwin_mut().w_vsep_width, &mut wp.w_vsep_width);
    std::mem::swap(&mut curwin_mut().w_hsep_height, &mut wp.w_hsep_height);

    frame_fix_height(curwin_mut());
    frame_fix_height(&mut *wp);
    frame_fix_width(curwin_mut());
    frame_fix_width(&mut *wp);

    win_comp_pos();

    if g_st().curbuf != wp.w_buffer {
        reset_visual_and_resel();
    } else if g_st().visual_global.active {
        wp.w_cursor = curwin_mut().w_cursor;
    }

    win_enter(wp, true);
    redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
}

fn win_rotate(upwards: bool, mut count: i32) {
    if curwin_mut().w_floating {
        emsg(E_FLOATEXCHANGE);
        return;
    }

    if count <= 0 || one_window(curwin_mut(), None) {
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

            frame_append(frp.borrow(), wp1.w_frame.borrow());

            wp2 = frp.fr_win.get();
        } else {
            frp = curwin_mut().w_frame.clone();
            while !frp.fr_next.get().is_null() {
                frp = frp.fr_next.get();
            }

            wp1 = frp.fr_win.get();
            wp2 = wp1.w_prev;

            win_remove(wp1.borrow_mut(), None);

            frame_remove(frp.borrow());

            debug_assert!(!frp.fr_parent.get().fr_child.get().is_null());

            win_append(
                frp.fr_parent
                    .get()
                    .fr_child
                    .get()
                    .fr_win
                    .get()
                    .opt()
                    .and_then(|w| w.w_prev.opt_mut()),
                wp1.borrow_mut(),
                None,
            );

            frame_insert(frp.fr_parent.get().fr_child.get().borrow(), frp.borrow());
        }

        std::mem::swap(
            &mut (wp2.borrow_mut()).w_status_height,
            &mut (wp1.borrow_mut()).w_status_height,
        );

        std::mem::swap(
            &mut (wp2.borrow_mut()).w_hsep_height,
            &mut (wp1.borrow_mut()).w_hsep_height,
        );

        frame_fix_height(wp1.borrow_mut());

        frame_fix_height(wp2.borrow_mut());

        std::mem::swap(
            &mut (wp2.borrow_mut()).w_vsep_width,
            &mut (wp1.borrow_mut()).w_vsep_width,
        );

        frame_fix_width(wp1.borrow_mut());

        frame_fix_width(wp2.borrow_mut());

        win_comp_pos();
    }

    (wp1.borrow_mut()).w_pos_changed = true;

    (wp2.borrow_mut()).w_pos_changed = true;

    redraw_all_later(UPD_NOT_VALID);
}

pub fn win_splitmove(wp_in: &mut WinInfoInfo, size: i32, flags: i32) -> bool {
    let wp = crate::globals::WinPtr::from(wp_in);
    let mut dir: i32 = 0;
    let height = wp.borrow_mut().w_height;

    if one_window(wp.borrow_mut(), None) {
        return true;
    }
    if is_ctx_win(wp.borrow()) || !check_split_disallowed(wp.opt()) {
        return false;
    }

    let mut unflat_altfr = crate::globals::FramePtr::NULL;
    if wp.borrow_mut().w_floating {
        win_remove(wp.borrow_mut(), None);
    } else {
        winframe_remove(wp.borrow_mut(), &mut dir, None, Some(&mut unflat_altfr));

        debug_assert!(!unflat_altfr.is_null());

        win_remove(wp.borrow_mut(), None);

        last_status(false);
        win_comp_pos();
    }

    if win_split_ins(size, flags, wp.opt_mut(), dir, unflat_altfr.opt()).is_none() {
        if !wp.borrow_mut().w_floating {
            debug_assert!(!unflat_altfr.is_null());

            winframe_restore(wp.borrow_mut(), dir, unflat_altfr.borrow());
        }

        win_append(wp.w_prev.opt_mut(), wp.borrow_mut(), None);

        return false;
    }

    if size == 0 && (flags & WSP_VERT) == 0 && win_valid(wp.opt()) && !wp.borrow_mut().w_floating {
        win_setheight_win(height, wp.borrow_mut(), true);

        if opt_vars().p_ea {
            win_equal(curwin_ptr().opt(), wp == curwin_ptr(), b'v' as i32);
        }
    }

    true
}

pub fn win_move_after(win1: &mut WinInfoInfo, win2: &mut WinInfoInfo) {
    if ptr::eq(win1, win2) {
        return;
    }

    if win2.w_next != win1 {
        if win1.frame_mut().fr_parent.get() != win2.frame_mut().fr_parent.get() {
            iemsg(c"INTERNAL: trying to move a window into another frame");
            return;
        }

        if g_st().lastwin == win1 {
            let prev = win1.prev_mut();
            std::mem::swap(&mut prev.w_status_height, &mut win1.w_status_height);
            std::mem::swap(&mut prev.w_hsep_height, &mut win1.w_hsep_height);

            if prev.w_vsep_width == 1 {
                prev.w_vsep_width = 0;
                prev.frame_mut()
                    .fr_width
                    .set(prev.frame_mut().fr_width.get() - 1);
                win1.w_vsep_width = 1;
                win1.frame_mut()
                    .fr_width
                    .set(win1.frame_mut().fr_width.get() + 1);
            }
        } else if g_st().lastwin == win2 {
            std::mem::swap(&mut win1.w_status_height, &mut win2.w_status_height);
            std::mem::swap(&mut win1.w_hsep_height, &mut win2.w_hsep_height);

            if win1.w_vsep_width == 1 {
                win2.w_vsep_width = 1;
                win2.frame_mut()
                    .fr_width
                    .set(win2.frame_mut().fr_width.get() + 1);
                win1.w_vsep_width = 0;
                win1.frame_mut()
                    .fr_width
                    .set(win1.frame_mut().fr_width.get() - 1);
            }
        }
        win_remove(&mut *win1, None);
        frame_remove(&win1.frame_mut());
        win_append(Some(&mut *win2), &mut *win1, None);
        frame_append(&win2.frame_mut(), &win1.frame_mut());

        win_comp_pos();
        redraw_later(Some(crate::globals::curwin_mut()), UPD_NOT_VALID);
    }
    win1.w_pos_changed = true;
    win2.w_pos_changed = true;

    win_enter(&mut *win1, false);
}

fn get_maximum_wincount(fr: &Frame, mut height: i32) -> i32 {
    if fr.fr_layout.get() != FR_COL {
        return height / (opt_vars().p_wmh as i32 + STATUS_HEIGHT + frame2win(fr).w_winbar_height);
    } else if global_winbar_height() != 0 {
        return height / (opt_vars().p_wmh as i32 + STATUS_HEIGHT + 1);
    }

    let mut total_wincount: i32 = 0;
    for frp in frame_iter_ptr(fr.fr_child.get()) {
        let wp = frame2win(&frp);
        if OptInt::from(height)
            < (opt_vars().p_wmh + OptInt::from(STATUS_HEIGHT + wp.w_winbar_height))
        {
            break;
        }
        height -= opt_vars().p_wmh as i32 + STATUS_HEIGHT + wp.w_winbar_height;
        total_wincount += 1;
    }

    total_wincount += height / (opt_vars().p_wmh as i32 + STATUS_HEIGHT);
    total_wincount
}

pub fn win_equal(next_curwin: Option<&WinInfoInfo>, current: bool, mut dir: i32) {
    let next_ptr = next_curwin.map_or(ptr::null_mut(), |w| ptr::from_ref(w).cast_mut());
    if dir == 0 {
        dir = i32::from(opt_vars().p_ead.first_byte());
    }
    win_equal_rec(
        crate::globals::WinPtr::new(if next_ptr.is_null() {
            curwin_ptr().as_ptr()
        } else {
            next_ptr
        }),
        current,
        &topframe_mut(),
        dir,
        0,
        tabline_height(),
        g_st().columns_global,
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
            || topfr.win_mut().w_winrow != row
            || topfr.fr_width.get() != width
            || topfr.win_mut().w_wincol != col
        {
            topfr.win_mut().w_winrow = row;
            frame_new_height(topfr, height, false, false, false);
            topfr.win_mut().w_wincol = col;
            frame_new_width(topfr, width, false, false);
            redraw_all_later(UPD_NOT_VALID);
        }
    } else if topfr.fr_layout.get() == FR_ROW {
        topfr.fr_width.set(width);
        topfr.fr_height.set(height);

        if dir != b'v' as i32 {
            let mut n = frame_minwidth(topfr, nowin());
            if col + width == g_st().columns_global {
                extra_sep = 1;
            } else {
                extra_sep = 0;
            }
            totwincount = (n + extra_sep) / (opt_vars().p_wmw as i32 + 1);
            has_next_curwin = frame_has_win(topfr, next_curwin.opt());

            let m = frame_minwidth(topfr, next_curwin);
            room = width - m;
            if room < 0 {
                next_curwin_size = opt_vars().p_wiw as i32 + room;
                room = 0;
            } else {
                next_curwin_size = -1;
                for fr in frame_iter_ptr(topfr.fr_child.get()) {
                    if frame_fixed_width(&fr) {
                        n = frame_minwidth(&fr, nowin());
                        let mut new_size = fr.fr_width.get();
                        if frame_has_win(&fr, next_curwin.opt()) {
                            room += opt_vars().p_wiw as i32 - opt_vars().p_wmw as i32;
                            next_curwin_size = 0;
                            new_size = new_size.max(opt_vars().p_wiw as i32);
                        } else {
                            totwincount -= (n + if fr.fr_next.get().is_null() {
                                extra_sep
                            } else {
                                0
                            }) / (opt_vars().p_wmw as i32 + 1);
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
                            > opt_vars().p_wiw
                    {
                        next_curwin_size = ((OptInt::from(room)
                            + opt_vars().p_wiw
                            + OptInt::from(totwincount - 1) * opt_vars().p_wmw
                            + OptInt::from(totwincount - 1))
                            as i32)
                            / totwincount;
                        room -= next_curwin_size - opt_vars().p_wiw as i32;
                    } else {
                        next_curwin_size = opt_vars().p_wiw as i32;
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
                }) / (opt_vars().p_wmw as i32 + 1);
                let m = frame_minwidth(&fr, next_curwin);
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
                    next_curwin_size -= opt_vars().p_wiw as i32 - (m - n);
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
                win_equal_rec(next_curwin, current, &fr, dir, col, row, new_size, height);
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
            if row + height >= g_st().cmdline_row && opt_vars().p_ls == 0 {
                extra_sep = STATUS_HEIGHT;
            } else if global_stl_height() > 0 {
                extra_sep = 1;
            } else {
                extra_sep = 0;
            }
            totwincount = get_maximum_wincount(topfr, n + extra_sep);
            has_next_curwin = frame_has_win(topfr, next_curwin.opt());

            let m = frame_minheight(topfr, next_curwin);
            room = height - m;
            if room < 0 {
                next_curwin_size = opt_vars().p_wh as i32 + room;
                room = 0;
            } else {
                next_curwin_size = -1;
                for fr in frame_iter_ptr(topfr.fr_child.get()) {
                    if frame_fixed_height(&fr) {
                        n = frame_minheight(&fr, nowin());
                        let mut new_size = fr.fr_height.get();
                        if frame_has_win(&fr, next_curwin.opt()) {
                            room += opt_vars().p_wh as i32 - opt_vars().p_wmh as i32;
                            next_curwin_size = 0;
                            new_size = new_size.max(opt_vars().p_wh as i32);
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
                            > opt_vars().p_wh
                    {
                        next_curwin_size = ((OptInt::from(room)
                            + opt_vars().p_wh
                            + OptInt::from(totwincount - 1) * opt_vars().p_wmh
                            + OptInt::from(totwincount - 1))
                            as i32)
                            / totwincount;
                        room -= next_curwin_size - opt_vars().p_wh as i32;
                    } else {
                        next_curwin_size = opt_vars().p_wh as i32;
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
                let m = frame_minheight(&fr, next_curwin);
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
                    next_curwin_size -= opt_vars().p_wh as i32 - (m - n);
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
                win_equal_rec(next_curwin, current, &fr, dir, col, row, width, new_size);
            }
            row += new_size;
            height -= new_size;
            totwincount -= wincount;
        }
    }
}

pub fn leaving_window(win: &mut WinInfoInfo) {
    if !bt_prompt(win.buf_opt()) || is_ctx_win(win) {
        return;
    }

    win.buf_mut().b_prompt_insert = g_st().restart_edit;
    if g_st().restart_edit != NUL as i32 && g_st().mode_displayed {
        g_st().clear_cmdline = true;
    }
    g_st().restart_edit = NUL as i32;

    if (g_st().state_global & MODE_INSERT) != 0 && !g_st().ins_global.stop_insert_mode {
        g_st().ins_global.stop_insert_mode = true;
        if win.buf_mut().b_prompt_insert == NUL as i32 {
            win.buf_mut().b_prompt_insert = b'A' as i32;
        }
    }
}

pub fn entering_window(win: &mut WinInfoInfo) {
    if !bt_prompt(win.buf_opt()) || is_ctx_win(win) {
        return;
    }

    if win.buf_mut().b_prompt_insert != NUL as i32 {
        g_st().ins_global.stop_insert_mode = false;
    }

    if (g_st().state_global & MODE_INSERT) == 0 {
        g_st().restart_edit = win.buf_mut().b_prompt_insert;
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
    win_init_empty(curwin_mut());
}

pub fn close_windows(buf_in: Option<&mut FileBuf>, keep_curwin: bool) {
    let buf = crate::globals::BufPtr::from(buf_in);
    g_st().redrawing_disabled += 1;

    let mut wp = g_st().lastwin;
    while !wp.is_null() && (is_ctx_win(lastwin_mut()) || !one_window(wp.borrow_mut(), None)) {
        if wp.borrow().w_buffer == buf
            && (!keep_curwin || wp != curwin_handle())
            && !(win_locked(wp.borrow_mut()) != 0 || wp.borrow().buf_mut().b_locked > 0)
        {
            if window_layout_locked(Cmdidx::CmdSize) {
                g_st().redrawing_disabled -= 1;
                return;
            }
            if !win_close(wp.borrow_mut(), false, false) {
                break;
            }
            wp = g_st().lastwin;
        } else {
            wp = wp.borrow().w_prev;
        }
    }

    let mut tp = g_st().first_tabpage;
    while !tp.is_null() {
        let mut nexttp = tp.borrow().tp_next;
        if tp.as_ptr() != g_st().curtab.as_ptr() {
            let mut wp2 = tp.borrow().tp_lastwin;
            while !wp2.is_null() {
                if wp2.borrow().w_buffer == buf
                    && !(win_locked(wp2.borrow_mut()) != 0 || wp2.borrow().buf_mut().b_locked > 0)
                {
                    if window_layout_locked(Cmdidx::CmdSize) {
                        g_st().redrawing_disabled -= 1;
                        return;
                    }
                    if !win_close_othertab(wp2.borrow_mut(), 0, tp.borrow_mut(), false) {
                        g_st().redrawing_disabled -= 1;
                        return;
                    }
                    nexttp = g_st().first_tabpage;
                    break;
                }
                wp2 = wp2.borrow().w_prev;
            }
        }
        tp = nexttp;
    }

    g_st().redrawing_disabled -= 1;
}

pub fn last_window(win: &mut WinInfoInfo) -> bool {
    one_window(win, None) && crate::globals::first_tabpage_mut().tp_next.is_null()
}

pub fn one_window(win: &mut WinInfoInfo, tp: Option<&TabPageInfo>) -> bool {
    let first = tp.map_or_else(|| g_st().firstwin.as_ptr(), |t| t.tp_firstwin.as_ptr());
    debug_assert!(
        tp.is_none_or(|t| g_st().curtab != t)
            && !crate::globals::WinPtr::new(first).borrow().w_floating
    );
    ptr::eq(first, (&raw mut *win)) && win.next_opt().is_none_or(|next| next.w_floating)
}

fn can_close_floating_windows(tp: Option<&TabPageInfo>) -> bool {
    debug_assert!(
        tp.is_none_or(|t| g_st().curtab != t) && (tp.is_some() || !is_ctx_win(lastwin_mut()))
    );
    let mut wp = tp.map_or_else(|| g_st().lastwin, |t| t.tp_lastwin);
    while wp.w_floating {
        let mut buf = wp.w_buffer;
        let need_hide = buf_is_changed_fn(&mut buf) && buf.b_nwindows <= 1;
        if need_hide && !buf_hide(&buf) {
            return false;
        }

        wp = wp.w_prev;
    }
    true
}

fn close_last_window_tabpage(
    win_in: &mut WinInfoInfo,
    mut free_buf: bool,
    prev_curtab_in: &mut TabPageInfo,
) -> bool {
    let win = crate::globals::WinPtr::from(win_in);
    let prev_curtab = crate::globals::TabpagePtr::from(prev_curtab_in);
    if !one_window_macro() {
        return false;
    }

    let old_curbuf = g_st().curbuf;
    let save_lastused = g_st().lastused_tabpage;

    let term = if !win.borrow_mut().w_buffer.is_null() {
        win.borrow_mut().buf_mut().terminal
    } else {
        crate::terminal::TermId::NULL
    };
    if !term.is_null() {
        free_buf = false;
    }

    goto_tabpage_tp(alt_tabpage(), false, !win.borrow_mut().w_buffer.is_null());

    if g_st().curtab != prev_curtab
        && valid_tabpage(prev_curtab.opt())
        && prev_curtab.borrow_mut().tp_firstwin == win
    {
        win_close_othertab(
            win.borrow_mut(),
            i32::from(free_buf),
            prev_curtab.borrow_mut(),
            false,
        );
    }

    if valid_tabpage(save_lastused.opt()) && g_st().curtab != save_lastused {
        g_st().lastused_tabpage = save_lastused;
    }
    entering_window(curwin_mut());

    apply_autocmds(EVENT_WINENTER, None, None, false, curbuf_opt_mut());
    apply_autocmds(EVENT_TABENTER, None, None, false, curbuf_opt_mut());
    if g_st().curbuf != old_curbuf {
        apply_autocmds(EVENT_BUFENTER, None, None, false, curbuf_opt_mut());
    }
    true
}

fn win_close_buffer(win_in: &mut WinInfoInfo, action: i32, abort_if_last: bool) -> bool {
    let win = crate::globals::WinPtr::from(win_in);
    if !win.borrow_mut().w_buffer.is_null() {
        reset_synblock(win.borrow_mut());
    }

    if !win.borrow_mut().w_buffer.is_null()
        && bt_quickfix(win.w_buffer.opt())
        && win.borrow_mut().buf_mut().b_nwindows == 1
    {
        win.borrow_mut().buf_mut().b_p_bl = false;
    }

    let mut retval = false;
    if !win.borrow_mut().w_buffer.is_null() {
        let mut bufref: Bufref = Bufref::new();
        set_bufref(&mut bufref, g_st().curbuf.opt_mut());

        win.borrow_mut().w_locked += 1;

        retval = close_buffer(
            win.opt_mut(),
            win.w_buffer.opt_mut(),
            action,
            abort_if_last,
            true,
            true,
        );

        if win_valid_any_tab(win.opt()) {
            win.borrow_mut().w_locked -= 1;
        }

        if !bufref_valid(Some(&bufref)) {
            g_st().curbuf = (g_st().firstbuf.as_ptr()).into();
        }
    }

    retval
}

fn win_unclose_buffer(win: &mut WinInfoInfo, _bufref: &mut Bufref) {
    if win.w_buffer.is_null() {
        win.w_buffer = (g_st().firstbuf.as_ptr()).into();
        win.buf_mut().b_nwindows += 1;
        if &raw mut *win == curwin_ptr() {
            g_st().curbuf = curwin_mut().w_buffer;
        }
        win_init_empty(win);
    }
}

pub fn win_close(win_in: &mut WinInfoInfo, free_buf: bool, force: bool) -> bool {
    let win = crate::globals::WinPtr::from(win_in);
    let prev_curtab = g_st().curtab;
    let win_frame = if win.borrow_mut().w_floating {
        crate::globals::FramePtr::NULL
    } else {
        win.borrow_mut().frame_mut().fr_parent.get()
    };
    let had_diffmode = win.borrow_mut().w_onebuf_opt.wo_diff;

    if last_window(win.borrow_mut()) {
        emsg(gettext(E_CANNOT_CLOSE_LAST_WINDOW));
        return false;
    }
    if !win.borrow_mut().w_floating && window_layout_locked(Cmdidx::CmdClose) {
        return false;
    }

    if win_locked(win.borrow_mut()) != 0
        || !win.borrow_mut().w_buffer.is_null() && win.borrow_mut().buf_mut().b_locked > 0
    {
        return false;
    }
    if is_ctx_win(win.borrow()) {
        emsg(gettext(E_AUTOCMD_CLOSE));
        return false;
    }
    if lastwin_mut().w_floating && one_window(win.borrow_mut(), None) {
        if is_ctx_win(lastwin_mut()) {
            emsg(gettext(
                c"E814: Cannot close window, only autocmd window would remain",
            ));
            return false;
        }
        if force || can_close_floating_windows(None) {
            while lastwin_mut().w_floating {
                let hide = !buf_hide(lastwin_mut().buf_mut());
                if !win_close(lastwin_mut(), hide, true) {
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

    if close_last_window_tabpage(win.borrow_mut(), free_buf, prev_curtab.borrow_mut()) {
        return false;
    }

    let mut help_window = false;
    let mut quickfix_window = false;

    if bt_help(win.w_buffer.opt()) {
        help_window = true;
    } else {
        clear_snapshot(curtab_mut(), SNAP_HELP_IDX as i32);
    }

    if bt_quickfix(win.w_buffer.opt()) {
        quickfix_window = true;
    } else {
        clear_snapshot(curtab_mut(), SNAP_QUICKFIX_IDX as i32);
    }

    let mut other_buffer = false;

    if win == curwin_ptr() {
        leaving_window(curwin_mut());

        let wp = if win.borrow_mut().w_floating {
            crate::globals::WinPtr::from(win_float_find_altwin(win.borrow(), None))
        } else {
            crate::globals::WinPtr::from(frame2win(&win_altframe(win.borrow_mut(), None)))
        };

        if wp.w_buffer != g_st().curbuf {
            reset_visual_and_resel();

            other_buffer = true;
            if !win_valid(win.opt()) {
                return false;
            }

            win.borrow_mut().w_locked += 1;

            apply_autocmds(EVENT_BUFLEAVE, None, None, false, curbuf_opt_mut());
            if !win_valid(win.opt()) {
                return false;
            }

            win.borrow_mut().w_locked -= 1;

            if last_window(win.borrow_mut()) {
                return false;
            }
        }

        win.borrow_mut().w_locked += 1;

        apply_autocmds(EVENT_WINLEAVE, None, None, false, curbuf_opt_mut());
        if !win_valid(win.opt()) {
            return false;
        }

        win.borrow_mut().w_locked -= 1;

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

    set_bufref(&mut bufref, win.w_buffer.opt_mut());

    win_close_buffer(
        win.borrow_mut(),
        if free_buf { DOBUF_UNLOAD } else { 0 },
        true,
    );

    if win_valid(win.opt())
        && win.borrow_mut().w_buffer.is_null()
        && !win.borrow_mut().w_floating
        && last_window(win.borrow_mut())
    {
        if curwin_mut().w_buffer.is_null() {
            curwin_mut().w_buffer = g_st().curbuf;
        }
        getout(0);
    }
    if g_st().curtab != prev_curtab
        && win_valid_any_tab(win.opt())
        && win.borrow_mut().w_buffer.is_null()
    {
        win_close_othertab(win.borrow_mut(), 0, prev_curtab.borrow_mut(), force);
        return false;
    }

    if !win_valid(win.opt()) {
        return false;
    }
    if one_window(win.borrow_mut(), None)
        && (crate::globals::first_tabpage_mut().tp_next.is_null() || lastwin_mut().w_floating)
    {
        if !crate::globals::first_tabpage_mut().tp_next.is_null() {
            emsg(E_FLOATONLY);
        }

        win_unclose_buffer(win.borrow_mut(), &mut bufref);

        return false;
    }
    if close_last_window_tabpage(win.borrow_mut(), free_buf, prev_curtab.borrow_mut()) {
        return false;
    }

    WIN_STATE.with(|s| s.split_disallowed.set(s.split_disallowed.get() + (1)));

    let was_floating = win.borrow_mut().w_floating;
    if ui_has(K_UIMULTIGRID) {
        ui_call_win_close(Integer::from(win.borrow_mut().w_grid_alloc.handle));
    }

    if win.borrow_mut().w_floating {
        ui_comp_remove_grid(&mut win.borrow_mut().w_grid_alloc);

        debug_assert!(!g_st().first_tabpage.is_null());
        if win.borrow_mut().w_config.external {
            for tp in for_all_tabpages_ptr() {
                if g_st().curtab != tp && tp.tp_curwin == win {
                    tp.tp_curwin = tp.tp_firstwin;
                }
            }
        }
    }

    set_bufref(&mut bufref, win.w_buffer.opt_mut());

    if !win.borrow_mut().w_buffer.is_null() {
        win.borrow_mut().buf_mut().b_nwindows -= 1;
    }

    let had_cmdline_ruler =
        opt_vars().p_ru && win == curwin_ptr() && win.borrow_mut().w_status_height == 0;
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
        set_curwin(wp);
        if (wp.borrow_mut()).w_onebuf_opt.wo_pvw || bt_quickfix(wp.w_buffer.opt()) {
            loop {
                if (wp.borrow_mut()).w_next.is_null() {
                    wp = g_st().firstwin;
                } else {
                    wp = wp.w_next;
                }
                if wp == curwin_ptr() {
                    break;
                }
                if !(wp.borrow_mut()).w_onebuf_opt.wo_pvw
                    && !bt_quickfix(wp.w_buffer.opt())
                    && !((wp.borrow_mut()).w_floating && !win_is_focusable(wp.borrow()))
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
            && opt_vars().p_ea
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
    } else if had_cmdline_ruler && (wp.borrow_mut()).w_status_height > 0 {
        g_st().redraw_cmdline = true;
    }
    if !bufref.br_buf.is_null() && bufref_valid(Some(&bufref)) && !bufref.br_buf.terminal.is_null()
    {
        terminal_check_size(bufref.br_buf.terminal.borrow_mut());
    }

    if close_curwin {
        win_enter_ext(
            wp.borrow_mut(),
            WEE_CURWIN_INVALID | WEE_TRIGGER_ENTER_AUTOCMDS | WEE_TRIGGER_LEAVE_AUTOCMDS,
        );
        if other_buffer {
            apply_autocmds(EVENT_BUFENTER, None, None, false, curbuf_opt_mut());
        }
    }

    if one_window_macro()
        && curwin_mut().w_locked != 0
        && curbuf_mut().b_locked_split != 0
        && !crate::globals::first_tabpage_mut().tp_next.is_null()
    {
        apply_autocmds(EVENT_TABLEAVE, None, None, false, curbuf_opt_mut());
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

    if diffopt_closeoff() && had_diffmode && g_st().curtab == prev_curtab {
        let mut diffcount: i32 = 0;
        for dwin in for_all_windows_mut() {
            if dwin.w_onebuf_opt.wo_diff {
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

fn trigger_winnewpre() {
    window_layout_lock();
    apply_autocmds(EVENT_WINNEWPRE, None, None, false, None);
    window_layout_unlock();
}

fn do_autocmd_winclosed(win: &mut WinInfoInfo) {
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
        win.w_buffer.opt_mut(),
        Some(win),
    );
    WIN_STATE.with(|s| s.winclosed_recursive.set(false));
}

pub fn trigger_tabclosedpre(tp_in: &mut TabPageInfo) {
    let tp = crate::globals::TabpagePtr::from(tp_in);
    let ptp = g_st().curtab;

    if !has_event(EVENT_TABCLOSEDPRE) || WIN_STATE.with(|s| s.tabclosedpre_recursive.get()) {
        return;
    }

    if valid_tabpage(tp.opt()) {
        goto_tabpage_tp(tp.opt_mut(), false, false);
    }
    WIN_STATE.with(|s| s.tabclosedpre_recursive.set(true));
    window_layout_lock();
    apply_autocmds(EVENT_TABCLOSEDPRE, None, None, false, None);
    window_layout_unlock();
    WIN_STATE.with(|s| s.tabclosedpre_recursive.set(false));
    if valid_tabpage(ptp.opt()) {
        goto_tabpage_tp(ptp.opt_mut(), false, false);
    } else {
        goto_tabpage_tp(g_st().first_tabpage.opt_mut(), false, false);
    }
}

pub fn win_close_othertab(
    win_in: &mut WinInfoInfo,
    free_buf: i32,
    tp_in: &mut TabPageInfo,
    force: bool,
) -> bool {
    let win = crate::globals::WinPtr::from(win_in);
    let tp = crate::globals::TabpagePtr::from(tp_in);
    debug_assert!(g_st().curtab != tp);

    if window_layout_locked(Cmdidx::CmdSize) {
        return false;
    }
    if win_locked(win.borrow_mut()) != 0
        || !win.borrow_mut().w_buffer.is_null() && win.borrow_mut().buf_mut().b_locked > 0
    {
        return false;
    }
    if is_ctx_win(win.borrow()) {
        emsg(gettext(E_AUTOCMD_CLOSE));
        return false;
    }

    if tp.tp_lastwin.borrow().w_floating && one_window(win.borrow_mut(), tp.opt()) {
        if force || can_close_floating_windows(tp.opt()) {
            loop {
                if !tp.tp_lastwin.borrow().w_floating {
                    break;
                }
                let last_w = tp.tp_lastwin.borrow_mut();
                let free_b = i32::from(!buf_hide(&last_w.w_buffer));
                if !win_close_othertab(last_w, free_b, tp.borrow_mut(), true) {
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

    if !win.borrow_mut().w_buffer.is_null() {
        do_autocmd_winclosed(win.borrow_mut());

        if !win_valid_any_tab(win.opt()) {
            return false;
        }
    }

    if ptr::eq(
        tp.borrow_mut().tp_firstwin.as_ptr(),
        tp.borrow_mut().tp_lastwin.as_ptr(),
    ) && !tp.borrow_mut().tp_did_tabclosedpre
    {
        trigger_tabclosedpre(tp.borrow_mut());

        if !win_valid_any_tab(win.opt()) {
            return false;
        }
    }

    let mut bufref: Bufref = Bufref::new();

    set_bufref(&mut bufref, win.w_buffer.opt_mut());

    if !win.borrow_mut().w_buffer.is_null() {
        close_buffer(
            win.opt_mut(),
            win.w_buffer.opt_mut(),
            if free_buf != 0 { DOBUF_UNLOAD } else { 0 },
            false,
            true,
            true,
        );
    }

    if !valid_tabpage(tp.opt()) || g_st().curtab == tp || !tabpage_win_valid(tp.opt(), win.opt()) {
        if win_valid_any_tab(win.opt()) {
            win_unclose_buffer(win.borrow_mut(), &mut bufref);
        }
        return false;
    }
    if tp.tp_lastwin.borrow().w_floating && one_window(win.borrow_mut(), tp.opt()) {
        emsg(E_FLOATONLY);
        if win_valid_any_tab(win.opt()) {
            win_unclose_buffer(win.borrow_mut(), &mut bufref);
        }
        return false;
    }

    let mut free_tp_idx: i32 = 0;

    if ptr::eq(
        tp.borrow_mut().tp_firstwin.as_ptr(),
        tp.borrow_mut().tp_lastwin.as_ptr(),
    ) {
        free_tp_idx = tabpage_index(tp.opt());
        let h = tabline_height();

        if g_st().first_tabpage == tp {
            g_st().first_tabpage = tp.tp_next;
        } else {
            let mut ptp = g_st().first_tabpage;
            while !ptp.is_null() && ptp.tp_next != tp {
                ptp = ptp.tp_next;
            }
            if ptp.is_null() {
                internal_error(c"win_close_othertab()");
                return false;
            }

            (ptp.borrow_mut()).tp_next = tp.borrow_mut().tp_next;
        }
        g_st().redraw_tabline = true;
        if h != tabline_height() {
            win_new_screen_rows();
        }
    }

    if ui_has(K_UIMULTIGRID) {
        ui_call_win_close(Integer::from(win.borrow_mut().w_grid_alloc.handle));
    }
    if win.borrow_mut().w_floating {
        ui_comp_remove_grid(&mut win.borrow_mut().w_grid_alloc);
    }

    set_bufref(&mut bufref, win.w_buffer.opt_mut());

    if !win.borrow_mut().w_buffer.is_null() {
        win.borrow_mut().buf_mut().b_nwindows -= 1;
    }

    let mut dir: i32 = 0;

    win_free_mem(win.borrow_mut(), &mut dir, tp.opt_mut());

    if !bufref.br_buf.is_null() && bufref_valid(Some(&bufref)) && !bufref.br_buf.terminal.is_null()
    {
        terminal_check_size(bufref.br_buf.terminal.borrow_mut());
    }
    if free_tp_idx > 0 {
        free_tabpage(tp.opt_mut());

        if has_event(EVENT_TABCLOSED) {
            let mut prev_idx = [0u8; NUMBUFLEN];
            format_to_buf!(&mut prev_idx[..], c"%i", free_tp_idx);
            let prev_idx_cstr = crate::memory::bytes_as_cstr(&prev_idx);
            let buf_opt = if !bufref.br_buf.is_null() && bufref_valid(Some(&bufref)) {
                bufref.br_buf.opt_mut()
            } else {
                curbuf_opt_mut()
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
    win_in: &mut WinInfoInfo,
    dirp: &mut i32,
    tp_in: Option<&mut TabPageInfo>,
) -> Option<&'static mut WinInfoInfo> {
    let win = crate::globals::WinPtr::from(win_in);
    let tp = crate::globals::TabpagePtr::from(tp_in);
    let wp: crate::globals::WinPtr;
    let mut win_tp = if tp.is_null() { g_st().curtab } else { tp };

    if !win.borrow_mut().w_floating {
        let frp = win.borrow_mut().w_frame.clone();

        wp = crate::globals::WinPtr::from(winframe_remove(
            win.borrow_mut(),
            &mut *dirp,
            tp.opt_mut(),
            None,
        ));

        frp.clear_links();
    } else {
        *dirp = b'h' as i32;

        wp = crate::globals::WinPtr::from(win_float_find_altwin(win.borrow(), tp.opt()));
    }
    win_free(win.borrow_mut(), tp.opt_mut());

    if win_tp.tp_curwin == win {
        win_tp.tp_curwin = wp;
    }
    if g_st().cmdline_win == win {
        g_st().cmdline_win = crate::globals::WinPtr::NULL;
    }

    wp.opt_mut()
}

pub fn win_free_all() {
    g_st().cmdwin_type = 0;
    g_st().cmdwin_buf = crate::globals::BufPtr::NULL;

    while !crate::globals::first_tabpage_mut().tp_next.is_null() {
        tabpage_close(true);
    }

    while !g_st().lastwin.is_null() && lastwin_mut().w_floating {
        let wp = g_st().lastwin;
        win_remove(lastwin_mut(), None);
        let mut dummy: i32 = 0;
        win_free_mem(wp.borrow_mut(), &mut dummy, None);
        for item in g_st().ctx_win_vec.iter_mut() {
            if item.cw_win == wp {
                item.cw_win = crate::globals::WinPtr::NULL;
            }
        }
    }

    for i in 0..g_st().ctx_win_vec.len() {
        let cw = g_st().ctx_win_vec.as_slice()[i].cw_win;
        if let Some(cw_mut) = cw.opt_mut() {
            let mut dummy: i32 = 0;
            win_free_mem(cw_mut, &mut dummy, None);
            g_st().ctx_win_vec.as_mut_slice()[i].cw_win = crate::globals::WinPtr::NULL;
        }
    }

    g_st().ctx_win_vec = Vec::new();

    while !g_st().firstwin.is_null() {
        let mut dummy: i32 = 0;
        win_free_mem(g_st().firstwin.borrow_mut(), &mut dummy, None);
    }

    set_curwin(ptr::null_mut());
}

pub fn winframe_remove(
    win: &mut WinInfoInfo,
    dirp: &mut i32,
    tp: Option<&mut TabPageInfo>,
    unflat_altfr: Option<&mut crate::globals::FramePtr>,
) -> Option<&'static mut WinInfoInfo> {
    let mut altfr = crate::globals::FramePtr::NULL;
    let wp = crate::globals::WinPtr::from(winframe_find_altwin(win, dirp, tp, Some(&mut altfr))?);

    let frp_close = win.frame_mut();
    let altfr_ref = altfr.borrow();

    WIN_STATE.with(|s| s.frame_locked.set(s.frame_locked.get() + (1)));

    let topleft = frame2win(&frp_close.parent_mut());
    let mut row = topleft.w_winrow;
    let mut col = topleft.w_wincol;

    if win.w_vsep_width == 0
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

    wp.opt_mut()
}

pub fn winframe_find_altwin(
    win: &mut WinInfoInfo,
    dirp: &mut i32,
    tp: Option<&mut TabPageInfo>,
    altfr: Option<&mut crate::globals::FramePtr>,
) -> Option<&'static mut WinInfoInfo> {
    debug_assert!(tp.as_deref().is_none_or(|t| g_st().curtab != t));

    if one_window(win, tp.as_deref()) {
        return None;
    }

    let frp_close = win.w_frame.clone();
    let mut frp2 = win_altframe(win, tp);
    let mut wp = crate::globals::WinPtr::from(frame2win(frp2.borrow()));

    if frp_close.fr_parent.get().fr_layout.get() == FR_COL {
        if !frp2.fr_win.get().is_null() && frp2.fr_win.get().w_onebuf_opt.wo_wfh {
            let mut frp = frp_close.fr_prev.get();
            let mut frp3 = frp_close.fr_next.get();
            while !frp.is_null() || !frp3.is_null() {
                if !frp.is_null() {
                    if !frame_fixed_height(&frp) {
                        frp2 = frp;
                        wp = crate::globals::WinPtr::from(frame2win(frp2.borrow()));
                        break;
                    }

                    frp = frp.fr_prev.get();
                }
                if !frp3.is_null() {
                    if !frp3.fr_win.get().is_null() && !frp3.fr_win.get().w_onebuf_opt.wo_wfh {
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
        if !frp2.fr_win.get().is_null() && frp2.fr_win.get().w_onebuf_opt.wo_wfw {
            let mut frp = frp_close.fr_prev.get();
            let mut frp3 = frp_close.fr_next.get();
            while !frp.is_null() || !frp3.is_null() {
                if !frp.is_null() {
                    if !frame_fixed_width(&frp) {
                        frp2 = frp;
                        wp = crate::globals::WinPtr::from(frame2win(frp2.borrow()));
                        break;
                    }

                    frp = frp.fr_prev.get();
                }
                if !frp3.is_null() {
                    if !frp3.fr_win.get().is_null() && !frp3.fr_win.get().w_onebuf_opt.wo_wfw {
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

    debug_assert!(wp != &*win && frp2 != frp_close);
    if let Some(out) = altfr {
        *out = frp2;
    }

    wp.opt_mut()
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
        frp.fr_win.get().borrow_mut().w_frame = frp.fr_parent.get();
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

pub fn winframe_restore(wp: &mut WinInfoInfo, dir: i32, unflat_altfr: &Frame) {
    let frp = wp.frame_mut();

    if let Some(prev) = frp.prev_opt() {
        frame_append(&prev, &frp);
    } else {
        frame_insert(&frp.next_mut(), &frp);
    }

    if wp.w_vsep_width == 0
        && frp.parent_mut().fr_layout.get() == FR_ROW
        && !frp.fr_prev.get().is_null()
    {
        frame_set_vsep(&frp.prev_mut(), true);
    }

    if frp.parent_mut().fr_layout.get() == FR_COL && !frp.fr_prev.get().is_null() {
        if global_stl_height() == 0 && wp.w_status_height == 0 {
            frame_add_statusline(&frp.prev_mut());
        } else if global_stl_height() > 0 && wp.w_hsep_height == 0 {
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
        let mut row = topleft.w_winrow;
        let mut col = topleft.w_wincol;

        frame_comp_pos(&frp.parent_mut(), &mut row, &mut col);
    }
}

fn win_altframe(win: &mut WinInfoInfo, tp: Option<&mut TabPageInfo>) -> crate::globals::FramePtr {
    debug_assert!(tp.as_deref().is_none_or(|t| g_st().curtab != t));

    if one_window(win, tp.as_deref()) {
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
        && opt_vars().p_sb
    {
        target_fr = frp.fr_prev.get();
        other_fr = frp.fr_next.get();
    }

    if !frp.fr_parent.get().is_null()
        && frp.parent_mut().fr_layout.get() == FR_ROW
        && opt_vars().p_spr
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

fn alt_tabpage() -> Option<&'static mut TabPageInfo> {
    if (opt_vars().tcl_flags & K_OPT_TCL_FLAG_USELAST) != 0
        && valid_tabpage(g_st().lastused_tabpage.opt())
    {
        return g_st().lastused_tabpage.opt_mut();
    }

    let forward = !curtab_mut().tp_next.is_null()
        && ((opt_vars().tcl_flags & K_OPT_TCL_FLAG_LEFT) == 0
            || ptr::eq(g_st().curtab.as_ptr(), g_st().first_tabpage.as_ptr()));

    if forward {
        curtab_mut().tp_next.opt_mut()
    } else {
        for tp in for_all_tabpages_ptr() {
            if tp.tp_next == g_st().curtab {
                return Some(tp);
            }
        }
        g_st().first_tabpage.opt_mut()
    }
}

pub fn frame2win(frp: &Frame) -> &'static mut WinInfoInfo {
    let mut fr = crate::globals::FramePtr::from(frp);
    while fr.fr_win.get().is_null() {
        fr = fr.fr_child.get();
    }
    fr.fr_win.get().borrow_mut()
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

fn is_bottom_win(wp: &mut WinInfoInfo) -> bool {
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
            .max(opt_vars().p_ch + OptInt::from(topfrp.fr_height.get() - height));
        if new_ch != opt_vars().p_ch {
            let save_ch = WIN_STATE.with(|s| s.min_set_ch.get());
            set_option_value(K_OPT_CMDHEIGHT, integer_obj(new_ch), 0, true, None);
            WIN_STATE.with(|s| s.min_set_ch.set(save_ch));
        }
        height = (rows_avail() as i32).min(height);
    }
    if let Some(wp) = topfrp.win_opt() {
        if is_bottom_win(wp) {
            wp.w_hsep_height = 0;
        }
        win_new_height(wp, height - wp.w_hsep_height - wp.w_status_height);
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
        return win.w_onebuf_opt.wo_wfh;
    }
    if fr.fr_layout.get() == FR_ROW {
        return frame_iter_mut(fr.fr_child.get()).any(|child| frame_fixed_height(&child));
    }
    frame_iter_mut(fr.fr_child.get()).all(|child| frame_fixed_height(&child))
}

fn frame_fixed_width(fr: &Frame) -> bool {
    if let Some(win) = fr.win_opt() {
        return win.w_onebuf_opt.wo_wfw;
    }
    if fr.fr_layout.get() == FR_COL {
        return frame_iter_mut(fr.fr_child.get()).any(|child| frame_fixed_width(&child));
    }
    frame_iter_mut(fr.fr_child.get()).all(|child| frame_fixed_width(&child))
}

fn frame_add_statusline(frp: &Frame) {
    let mut fr = crate::globals::FramePtr::from(frp);
    if fr.fr_layout.get() == FR_LEAF {
        fr.win_mut().w_status_height = STATUS_HEIGHT;
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
            wp.borrow_mut().w_vsep_width = 0;
        }
        let new_w = width - wp.borrow().w_vsep_width;
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
                let w = frame_minwidth(frp.borrow(), crate::globals::WinPtr::NULL);
                if frp.fr_width.get() + extra_cols < w {
                    extra_cols += frp.fr_width.get() - w;
                    frame_new_width(frp.borrow(), w, leftfirst, wfw);
                } else {
                    let new_w = frp.fr_width.get() + extra_cols;
                    frame_new_width(frp.borrow(), new_w, leftfirst, wfw);
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
        if add && wp.w_vsep_width == 0 {
            if wp.w_width > 0 {
                win_new_width(wp, wp.w_width - 1);
            }
            wp.w_vsep_width = 1;
        } else if !add && wp.w_vsep_width == 1 {
            win_new_width(wp, wp.w_width + 1);
            wp.w_vsep_width = 0;
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
        fr.win_mut().w_hsep_height = 1;
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
fn frame_fix_width(wp: &mut WinInfoInfo) {
    wp.frame_mut().fr_width.set(wp.w_width + wp.w_vsep_width);
}

#[inline]
fn frame_fix_height(wp: &mut WinInfoInfo) {
    wp.frame_mut()
        .fr_height
        .set(wp.w_height + wp.w_hsep_height + wp.w_status_height);
}

fn frame_minheight(topfrp: &Frame, next_curwin: crate::globals::WinPtr) -> i32 {
    let mut m: i32;
    if let Some(win) = topfrp.win_opt() {
        let extra_height = win.w_winbar_height + win.w_hsep_height + win.w_status_height;
        if topfrp.fr_win.get() == next_curwin.as_ptr() {
            m = opt_vars().p_wh as i32 + extra_height;
        } else {
            m = opt_vars().p_wmh as i32 + extra_height;
            if topfrp.fr_win.get() == curwin_ptr() && next_curwin.is_null() && opt_vars().p_wmh == 0
            {
                m += 1;
            }
        }
    } else if topfrp.fr_layout.get() == FR_ROW {
        m = 0;
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            let n = frame_minheight(&frp, next_curwin);
            if n > m {
                m = n;
            }
        }
    } else {
        m = 0;
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            m += frame_minheight(&frp, next_curwin);
        }
    }
    m
}

fn frame_minwidth(topfrp: &Frame, next_curwin: crate::globals::WinPtr) -> i32 {
    let mut m: i32;
    if let Some(win) = topfrp.win_opt() {
        if topfrp.fr_win.get() == next_curwin.as_ptr() {
            m = opt_vars().p_wiw as i32 + win.w_vsep_width;
        } else {
            m = opt_vars().p_wmw as i32 + win.w_vsep_width;
            if opt_vars().p_wmw == 0 && topfrp.fr_win.get() == curwin_ptr() && next_curwin.is_null()
            {
                m += 1;
            }
        }
    } else if topfrp.fr_layout.get() == FR_COL {
        m = 0;
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            let n = frame_minwidth(&frp, next_curwin);
            m = m.max(n);
        }
    } else {
        m = 0;
        for frp in frame_iter_mut(topfrp.fr_child.get()) {
            m += frame_minwidth(&frp, next_curwin);
        }
    }
    m
}

pub fn close_others(message: i32, forceit: bool, ignore_pinned: bool) {
    let old_curwin = curwin_ptr();

    if curwin_mut().w_floating {
        if message != 0 && !g_st().autocmd_busy {
            emsg(E_FLOATONLY);
        }
        return;
    }

    if one_window(firstwin_mut(), None) && !lastwin_mut().w_floating {
        if message != 0 && !g_st().autocmd_busy {
            msg(gettext(M_ONLYONE), 0);
        }
        return;
    }

    let mut nextwp: crate::globals::WinPtr;
    let mut wp = g_st().firstwin;
    while win_valid(wp.opt()) {
        nextwp = wp.borrow().w_next;

        if old_curwin != curwin_ptr() && win_valid(old_curwin.opt()) {
            set_curwin(old_curwin);
            g_st().curbuf = curwin_mut().w_buffer;
        }

        if wp == curwin_handle() || (wp.borrow().w_onebuf_opt.wo_wp && !ignore_pinned) {
            wp = nextwp;
            continue;
        }

        if !buf_valid(wp.borrow().buf_opt()) && win_valid(wp.opt()) {
            wp.borrow_mut().w_buffer = crate::globals::BufPtr::NULL;
            win_close(wp.borrow_mut(), false, false);
            wp = nextwp;
            continue;
        }

        let r = can_abandon(wp.borrow().buf_mut(), forceit);
        if !win_valid(wp.opt()) {
            wp = g_st().firstwin;
            continue;
        }
        if !r {
            if message != 0
                && (opt_vars().p_confirm || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
                && opt_vars().p_write
            {
                dialog_changed(wp.borrow().buf_mut(), false);
                if !win_valid(wp.opt()) {
                    wp = g_st().firstwin;
                    continue;
                }
            }
            if buf_is_changed_fn(wp.borrow().buf_mut()) {
                wp = nextwp;
                continue;
            }
        }
        let close_buf =
            !buf_hide(wp.borrow().buf_mut()) && !buf_is_changed_fn(wp.borrow().buf_mut());
        win_close(wp.borrow_mut(), close_buf, false);
        wp = nextwp;
    }

    if message != 0 && !one_window_macro() {
        let mut has_non_pinned = false;
        for wp2 in for_all_windows_ptr() {
            if wp2 != curwin_ptr() && !wp2.w_onebuf_opt.wo_wp {
                has_non_pinned = true;
                break;
            }
        }
        if has_non_pinned {
            emsg(gettext(c"E445: Other window contains changes"));
        }
    }
}

pub fn unuse_tabpage(tp: &mut TabPageInfo) {
    tp.tp_topframe = g_st().topframe.clone();
    tp.tp_firstwin = g_st().firstwin;
    tp.tp_lastwin = g_st().lastwin;
    tp.tp_curwin = crate::globals::curwin_ptr();
    tp.tp_ch_used = opt_vars().p_ch;
}

pub fn use_tabpage(tp: &mut TabPageInfo) {
    g_st().curtab = (tp).into();
    g_st().topframe = curtab_mut().tp_topframe.clone();
    g_st().firstwin = (curtab_mut().tp_firstwin.as_ptr()).into();
    g_st().lastwin = (curtab_mut().tp_lastwin.as_ptr()).into();
    set_curwin(curtab_mut().tp_curwin.as_ptr());
    opt_vars().p_ch = curtab_mut().tp_ch_used;
}

pub fn win_alloc_first() {
    if !win_alloc_firstwin(None) {
        std::process::abort();
    }

    g_st().first_tabpage = (alloc_tabpage()).into();
    g_st().curtab = g_st().first_tabpage;
    unuse_tabpage(crate::globals::first_tabpage_mut());
}

pub fn win_alloc_ctx_win(idx: i32) {
    let mut err = ERROR_INIT;
    let mut fconfig = WIN_CONFIG_INIT;
    fconfig.width = g_st().columns_global;
    fconfig.height = 5;
    fconfig.focusable = false;
    fconfig.mouse = false;
    fconfig.hide = true;
    let cw_ref = win_new_float(None, true, fconfig, &mut err).unwrap();
    g_st().ctx_win_vec.as_mut_slice()[idx as usize].cw_win =
        crate::globals::WinPtr::from(&mut *cw_ref);
    cw_ref.buf_mut().b_nwindows -= 1;
    reset_binding(cw_ref);
}

fn win_alloc_firstwin(oldwin: Option<&mut WinInfoInfo>) -> bool {
    set_curwin(win_alloc(None, false));
    if let Some(oldwin) = oldwin {
        win_init(curwin_mut(), oldwin, 0);
        reset_binding(curwin_mut());
    } else {
        g_st().curbuf = crate::globals::BufPtr::from(buflist_new(None, None, 1, BLN_LISTED));
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
    g_st().topframe = curwin_mut().w_frame.clone();
    crate::globals::topframe_mut()
        .fr_width
        .set(g_st().columns_global);
    crate::globals::topframe_mut()
        .fr_height
        .set(g_st().rows_global - opt_vars().p_ch as i32 - global_stl_height());

    true
}

fn new_frame(wp: &mut WinInfoInfo) {
    let fr = Frame::new_rc();
    fr.fr_layout.set(FR_LEAF);
    fr.fr_win.set(crate::globals::WinPtr::from(&mut *wp));
    wp.w_frame = crate::globals::FramePtr::from(fr);
}

pub fn win_init_size() {
    let avail = rows_avail() as i32;
    firstwin_mut().w_height = avail;
    firstwin_mut().w_prev_height = avail;
    firstwin_mut().w_view_height = firstwin_mut().w_height - firstwin_mut().w_winbar_height;
    firstwin_mut().w_height_outer = firstwin_mut().w_height;
    firstwin_mut().w_winrow_off = firstwin_mut().w_winbar_height;
    crate::globals::topframe_mut().fr_height.set(avail);
    firstwin_mut().w_width = g_st().columns_global;
    firstwin_mut().w_view_width = firstwin_mut().w_width;
    firstwin_mut().w_width_outer = firstwin_mut().w_width;
    crate::globals::topframe_mut()
        .fr_width
        .set(g_st().columns_global);
}

fn alloc_tabpage() -> &'static mut TabPageInfo {
    let tp = Box::leak(Box::new(TabPageInfo::new()));
    WIN_STATE.with(|s| s.last_tp_handle.set(s.last_tp_handle.get() + (1)));
    tp.handle = WIN_STATE.with(|s| s.last_tp_handle.get());
    g_st().tabpage_handles.insert(tp.handle, tp.into());

    tp.tp_winvar = crate::eval::typval::tv_dict_item_alloc_static().di_id;
    init_var_dict(tv_dict_alloc(), tp.tp_winvar.borrow_mut(), VAR_SCOPE);
    tp.tp_ch_used = opt_vars().p_ch;

    &mut *tp
}

impl Drop for TabPageInfo {
    fn drop(&mut self) {
        g_st().tabpage_handles.remove(&self.handle);
        diff_clear(self);
        for idx in 0..SNAP_COUNT {
            clear_snapshot(self, idx as i32);
        }
        if let Some(tp_winvar) = self.tp_winvar.opt_mut() {
            if !tp_winvar.di_tv.vval.v_dict().is_null() {
                vars_clear(&mut self.tp_vars().dv_hashtab);
                unref_var_dict(&mut *self.tp_vars());
            }
            crate::eval::typval::tv_dict_item_free_slot(tp_winvar);
        }
        if g_st().lastused_tabpage == self {
            g_st().lastused_tabpage = crate::globals::TabpagePtr::NULL;
        }
    }
}

pub fn free_tabpage(tp: Option<&mut TabPageInfo>) {
    if let Some(tp_ref) = tp {
        crate::globals::TabpagePtr::from(tp_ref).drop_box();
    }
}

pub fn win_new_tabpage(
    after: i32,
    filename_opt: Option<&CStr>,
    enter: bool,
    first: Option<&mut Option<&'static mut WinInfoInfo>>,
) -> Option<&'static mut TabPageInfo> {
    {
        let old_curtab = g_st().curtab;

        if window_layout_locked(Cmdidx::CmdTabnew) {
            return None;
        }

        let mut newtp = crate::globals::TabpagePtr::from(&*alloc_tabpage());

        if enter {
            if !leave_tabpage(g_st().curbuf.opt_mut(), true) {
                free_tabpage(newtp.opt_mut());
                return None;
            }
        } else {
            unuse_tabpage(curtab_mut());
            curtab_mut().tp_old_rows_avail = rows_avail();
            g_st().firstwin = crate::globals::WinPtr::NULL;
            g_st().lastwin = crate::globals::WinPtr::NULL;
        }

        newtp.tp_localdir = old_curtab.tp_localdir.clone();

        g_st().curtab = newtp;

        let result = win_alloc_firstwin(old_curtab.tp_curwin.opt_mut());
        debug_assert!(result);
        if let Some(first_out) = first {
            *first_out = curwin_ptr().opt_mut();
        }

        if after == 1 {
            newtp.tp_next = g_st().first_tabpage;
            g_st().first_tabpage = newtp;
        } else {
            let mut tp = old_curtab;
            if after > 0 {
                let mut n: i32 = 2;
                tp = g_st().first_tabpage;
                while !tp.tp_next.is_null() && n < after {
                    n += 1;
                    tp = tp.tp_next;
                }
            }
            newtp.tp_next = tp.tp_next;
            tp.tp_next = newtp;
        }
        newtp.tp_firstwin = crate::globals::curwin_ptr();
        newtp.tp_lastwin = crate::globals::curwin_ptr();
        newtp.tp_curwin = crate::globals::curwin_ptr();

        win_init_size();
        firstwin_mut().w_winrow = tabline_height();
        firstwin_mut().w_prev_winrow = firstwin_mut().w_winrow;
        win_comp_scroll(curwin_mut());

        newtp.tp_topframe = g_st().topframe.clone();
        last_status(false);

        if !curbuf_mut().terminal.is_null() {
            terminal_check_size(curbuf_mut().terminal_mut());
        }

        if enter {
            redraw_all_later(UPD_NOT_VALID);
            tabpage_check_windows(old_curtab.borrow_mut());
            g_st().lastused_tabpage = old_curtab;
            entering_window(curwin_mut());

            apply_autocmds(EVENT_WINNEW, None, None, false, curbuf_opt_mut());
            apply_autocmds(EVENT_WINENTER, None, None, false, curbuf_opt_mut());
            apply_autocmds(
                EVENT_TABNEW,
                filename_opt,
                filename_opt,
                false,
                curbuf_opt_mut(),
            );
            apply_autocmds(EVENT_TABENTER, None, None, false, curbuf_opt_mut());
        } else {
            unuse_tabpage(curtab_mut());
            use_tabpage(old_curtab.borrow_mut());
            g_st().redraw_tabline = true;
            if curtab_mut().tp_old_rows_avail != rows_avail() {
                win_new_screen_rows();
            }

            let mut switchwin: CtxSwitch = CtxSwitch::new();
            let sw_ok = ctx_switch(
                &mut switchwin,
                newtp.tp_curwin.opt_mut(),
                newtp.opt_mut(),
                None,
                K_CTX_NO_DISPLAY,
            );
            debug_assert!(sw_ok);

            apply_autocmds(EVENT_WINNEW, None, None, false, curbuf_opt_mut());
            apply_autocmds(
                EVENT_TABNEW,
                filename_opt,
                filename_opt,
                false,
                curbuf_opt_mut(),
            );

            ctx_restore(&mut switchwin);
        }

        newtp.opt_mut()
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
    let status = win_new_tabpage(n, None, true, None).is_some();
    if status {
        apply_autocmds(EVENT_TABNEWENTERED, None, None, false, curbuf_opt_mut());
    }
    status
}

pub fn make_tabpages(maxcount: i32) -> i32 {
    let count = maxcount.min(opt_vars().p_tpm as i32);

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
    let mut tp = g_st().first_tabpage;
    while let Some(t) = tp.opt() {
        if tp == tpc_ptr {
            return true;
        }
        tp = t.tp_next;
    }
    false
}

pub fn valid_tabpage_win(tpc: &mut TabPageInfo) -> i32 {
    for tp in for_all_tabpages_ptr() {
        if ptr::eq(tp, tpc) {
            for wp in win_iter_ptr(if g_st().curtab == tp {
                g_st().firstwin.as_ptr()
            } else {
                tp.tp_firstwin.as_ptr()
            }) {
                if win_valid_any_tab(Some(&*wp)) {
                    return 1;
                }
            }
            return 0;
        }
    }
    0
}

pub fn close_tabpage(tab: &mut TabPageInfo) {
    let tab_ptr = crate::globals::TabpagePtr::from(tab);
    let ptp = if g_st().first_tabpage == tab_ptr {
        g_st().first_tabpage = tab_ptr.tp_next;
        g_st().first_tabpage
    } else {
        let mut prev_tp: Option<&mut TabPageInfo> = None;
        for tp in for_all_tabpages_mut() {
            if tp.tp_next == tab_ptr {
                prev_tp = Some(tp);
                break;
            }
        }
        let prev = prev_tp.expect("tabpage in list");
        prev.tp_next = tab_ptr.tp_next;
        crate::globals::TabpagePtr::from(&*prev)
    };

    goto_tabpage_tp(ptp.opt_mut(), false, false);
    free_tabpage(tab_ptr.opt_mut());
}

pub fn find_tabpage(n: i32) -> Option<&'static mut TabPageInfo> {
    if n == 0 {
        return g_st().curtab.opt_mut();
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

fn leave_tabpage(new_curbuf_in: Option<&mut FileBuf>, trigger_leave_autocmds: bool) -> bool {
    let new_curbuf = crate::globals::BufPtr::from(new_curbuf_in);
    let tp = curtab_mut();
    let tp_ptr = crate::globals::TabpagePtr::from(&*tp);

    leaving_window(curwin_mut());
    reset_visual_and_resel();
    if trigger_leave_autocmds {
        if g_st().curbuf != new_curbuf {
            apply_autocmds(EVENT_BUFLEAVE, None, None, false, curbuf_opt_mut());
            if g_st().curtab != tp_ptr {
                return false;
            }
        }
        apply_autocmds(EVENT_WINLEAVE, None, None, false, curbuf_opt_mut());
        if g_st().curtab != tp_ptr {
            return false;
        }
        apply_autocmds(EVENT_TABLEAVE, None, None, false, curbuf_opt_mut());
        if g_st().curtab != tp_ptr {
            return false;
        }
    }

    let tp = tp_ptr.borrow_mut();
    reset_dragwin();
    tp.tp_curwin = crate::globals::curwin_ptr();
    tp.tp_prevwin = g_st().prevwin;
    tp.tp_firstwin = g_st().firstwin;
    tp.tp_lastwin = g_st().lastwin;
    tp.tp_old_rows_avail = rows_avail();
    if tp.tp_old_columns != -1 {
        tp.tp_old_columns = i64::from(g_st().columns_global);
    }
    g_st().firstwin = crate::globals::WinPtr::NULL;
    g_st().lastwin = crate::globals::WinPtr::NULL;
    true
}

fn enter_tabpage(
    tp_in: &mut TabPageInfo,
    old_curbuf_in: Option<&mut FileBuf>,
    trigger_enter_autocmds: bool,
    trigger_leave_autocmds: bool,
) {
    let tp_ptr = crate::globals::TabpagePtr::from(tp_in);
    let old_curbuf = crate::globals::BufPtr::from(old_curbuf_in);
    let tp = tp_ptr.borrow_mut();
    let old_off = tp.firstwin_mut().w_winrow;
    let next_prevwin = tp.tp_prevwin.as_ptr();
    let old_curtab = curtab_mut();
    let old_curtab_ptr = crate::globals::TabpagePtr::from(&*old_curtab);
    let prev_p_ch = opt_vars().p_ch;

    use_tabpage(tp);

    if g_st().curtab != old_curtab_ptr && opt_vars().p_ch != prev_p_ch {
        tabpage_check_windows(old_curtab_ptr.borrow_mut());
        let new_ch = opt_vars().p_ch;
        opt_vars().p_ch = prev_p_ch;
        WIN_STATE.with(|s| s.command_frame_height.set(false));
        set_option_value(K_OPT_CMDHEIGHT, integer_obj(new_ch), 0, true, None);
        WIN_STATE.with(|s| s.command_frame_height.set(true));
    } else if g_st().curtab != old_curtab_ptr {
        tabpage_check_windows(old_curtab_ptr.borrow_mut());
    }

    let tp_curwin = tp_ptr.tp_curwin;
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
    g_st().prevwin = (next_prevwin).into();

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

    g_st().lastused_tabpage = old_curtab_ptr;

    if trigger_enter_autocmds {
        apply_autocmds(EVENT_TABENTER, None, None, false, curbuf_opt_mut());
        if g_st().curbuf != old_curbuf {
            apply_autocmds(EVENT_BUFENTER, None, None, false, curbuf_opt_mut());
        }
    }

    redraw_all_later(UPD_NOT_VALID);
}

fn tabpage_check_windows(old_curtab: &mut TabPageInfo) {
    for wp in win_iter_ptr(old_curtab.tp_firstwin.as_ptr()) {
        if wp.w_floating {
            if wp.w_config.external {
                win_remove(&mut *wp, Some(&mut *old_curtab));
                win_append(Some(lastwin_nofloating(None)), &mut *wp, None);
            } else {
                ui_comp_remove_grid(&mut wp.w_grid_alloc);
            }
        }
        wp.w_pos_changed = true;
    }

    for wp2 in for_all_windows_mut() {
        if wp2.w_floating && !wp2.w_config.external {
            win_config_float(wp2, wp2.w_config.clone());
        }
        wp2.w_pos_changed = true;
    }
}

pub fn goto_tabpage(n: i32) {
    if text_locked() {
        text_locked_msg();
        return;
    }

    if crate::globals::first_tabpage_mut().tp_next.is_null() {
        if n > 1 {
            beep_flush();
        }
        return;
    }

    let mut tp = crate::globals::TabpagePtr::NULL;

    if n == 0 {
        if curtab_mut().tp_next.is_null() {
            tp = g_st().first_tabpage;
        } else {
            tp = curtab_mut().tp_next;
        }
    } else if n < 0 {
        let mut ttp = g_st().curtab;
        for _ in n..0 {
            let mut cur = crate::globals::first_tabpage_mut();
            while cur.tp_next != ttp
                && let Some(next) = cur.next_opt()
            {
                cur = next;
            }
            tp = crate::globals::TabpagePtr::from(&*cur);
            ttp = tp;
        }
    } else if n == 9999 {
        if let Some(last) = for_all_tabpages_ptr().last() {
            tp = crate::globals::TabpagePtr::from(&*last);
        }
    } else {
        tp = crate::globals::TabpagePtr::from(find_tabpage(n));
        if tp.is_null() {
            beep_flush();
            return;
        }
    }

    goto_tabpage_tp(tp.opt_mut(), true, true);
}

pub fn goto_tabpage_tp(
    tp_in: Option<&mut TabPageInfo>,
    trigger_enter_autocmds: bool,
    trigger_leave_autocmds: bool,
) {
    let tp = crate::globals::TabpagePtr::from(tp_in);
    set_keep_msg(None, 0);

    g_st().skip_win_fix_scroll = true;
    let Some(tp_ref) = tp.opt_mut() else {
        g_st().skip_win_fix_scroll = false;
        return;
    };
    let new_curbuf = tp_ref.curwin_mut().w_buffer;
    if g_st().curtab != tp && leave_tabpage(new_curbuf.opt_mut(), trigger_leave_autocmds) {
        if valid_tabpage(tp.opt()) {
            enter_tabpage(
                tp.borrow_mut(),
                g_st().curbuf.opt_mut(),
                trigger_enter_autocmds,
                trigger_leave_autocmds,
            );
        } else {
            enter_tabpage(
                curtab_mut(),
                g_st().curbuf.opt_mut(),
                trigger_enter_autocmds,
                trigger_leave_autocmds,
            );
        }
    }
    g_st().skip_win_fix_scroll = false;
}

pub fn goto_tabpage_lastused() -> bool {
    if !valid_tabpage(g_st().lastused_tabpage.opt()) {
        return false;
    }

    goto_tabpage_tp(g_st().lastused_tabpage.opt_mut(), true, true);
    true
}

pub fn goto_tabpage_win(tp_in: &mut TabPageInfo, wp_in: &mut WinInfoInfo) {
    let tp = crate::globals::TabpagePtr::from(tp_in);
    let wp = crate::globals::WinPtr::from(wp_in);
    goto_tabpage_tp(tp.opt_mut(), true, true);
    if g_st().curtab == tp && win_valid(wp.opt()) {
        win_enter(wp.borrow_mut(), true);
    }
}

pub fn tabpage_move(nr: i32) {
    debug_assert!(!g_st().curtab.is_null());

    if crate::globals::first_tabpage_mut().tp_next.is_null() {
        return;
    }

    if g_st().tabpage_move_disallowed != 0 {
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

    if g_st().curtab == tp || (nr > 0 && !tp.tp_next.is_null() && tp.tp_next == g_st().curtab) {
        return;
    }

    let old_nr = tabpage_index(g_st().curtab.opt());
    let tp_dst = tp;

    if ptr::eq(g_st().curtab.as_ptr(), g_st().first_tabpage.as_ptr()) {
        g_st().first_tabpage = (curtab_mut().tp_next.as_ptr()).into();
    } else {
        let Some(prev_tp) = for_all_tabpages_ptr().find(|tp2| tp2.tp_next == g_st().curtab) else {
            return;
        };
        prev_tp.tp_next = curtab_mut().tp_next;
    }

    if nr <= 0 {
        curtab_mut().tp_next = g_st().first_tabpage;
        g_st().first_tabpage = g_st().curtab;
    } else {
        curtab_mut().tp_next = tp_dst.tp_next;
        tp_dst.tp_next = g_st().curtab;
    }

    g_st().redraw_tabline = true;

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
                value: integer_obj(Integer::from(tabpage_index(g_st().curtab.opt()))),
            },
        ];
        let data = Dict::from_slice(&mut data_items[..2]);
        let mut obj = dict_obj(data);
        aucmd_defer(
            EVENT_TABMOVED,
            Some(crate::memory::bytes_as_cstr(&prev_idx)),
            None,
            AUGROUP_ALL,
            curbuf_mut(),
            Some(&mut obj),
        );
    }
}

pub fn win_goto(wp_in: &mut WinInfoInfo) {
    let wp = crate::globals::WinPtr::from(wp_in);
    if text_or_buf_locked() {
        beep_flush();
        return;
    }

    let wp_ref = wp.borrow_mut();
    if g_st().curbuf != wp_ref.w_buffer {
        reset_visual_and_resel();
    } else if g_st().visual_global.active {
        wp_ref.w_cursor = curwin_mut().w_cursor;
    }

    if !win_valid(wp.opt()) {
        return;
    }

    win_enter(wp.borrow_mut(), true);
}

pub fn win_find_tabpage(win: &WinInfoInfo) -> Option<&'static mut TabPageInfo> {
    let win_ptr = ptr::from_ref(win);
    for tp in for_all_tabpages_mut() {
        for wp in for_all_windows_in_tab_ptr(tp) {
            if ptr::eq(wp, win_ptr) {
                return Some(tp);
            }
        }
    }
    None
}

pub fn win_vert_neighbor(
    tp: &mut TabPageInfo,
    wp: &mut WinInfoInfo,
    up: bool,
    mut count: i32,
) -> Option<&'static mut WinInfoInfo> {
    if wp.w_floating {
        let res = if win_valid(g_st().prevwin.opt()) && !crate::globals::prevwin_mut().w_floating {
            g_st().prevwin
        } else {
            g_st().firstwin
        };
        return res.opt_mut();
    }

    let mut foundfr = wp.frame_mut();

    'outer: while count > 0 {
        count -= 1;
        let mut nfr: crate::globals::FramePtr;
        let mut fr = foundfr.clone();
        loop {
            if tp.tp_topframe == fr {
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
                while frame2win(&fr).w_wincol + fr.fr_width.get() <= wp.w_wincol + wp.w_wcol
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
    foundfr.fr_win.get().opt_mut()
}

fn win_goto_ver(up: bool, count: i32) {
    if let Some(win) = win_vert_neighbor(curtab_mut(), curwin_mut(), up, count) {
        win_goto(win);
    }
}

pub fn win_horz_neighbor(
    tp: &mut TabPageInfo,
    wp: &mut WinInfoInfo,
    left: bool,
    mut count: i32,
) -> Option<&'static mut WinInfoInfo> {
    if wp.w_floating {
        let res = if win_valid(g_st().prevwin.opt()) && !crate::globals::prevwin_mut().w_floating {
            g_st().prevwin
        } else {
            g_st().firstwin
        };
        return res.opt_mut();
    }

    let mut foundfr = wp.frame_mut();

    'outer: while count > 0 {
        count -= 1;
        let mut nfr: crate::globals::FramePtr;
        let mut fr = foundfr.clone();
        loop {
            if tp.tp_topframe == fr {
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
                while frame2win(&fr).w_winrow + fr.fr_height.get() <= wp.w_winrow + wp.w_wrow
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
    foundfr.fr_win.get().opt_mut()
}

fn win_goto_hor(left: bool, count: i32) {
    if let Some(win) = win_horz_neighbor(curtab_mut(), curwin_mut(), left, count) {
        win_goto(win);
    }
}

pub fn win_enter(wp: &mut WinInfoInfo, undo_sync: bool) {
    win_enter_ext(
        wp,
        (if undo_sync { WEE_UNDO_SYNC } else { 0 })
            | WEE_TRIGGER_ENTER_AUTOCMDS
            | WEE_TRIGGER_LEAVE_AUTOCMDS,
    );
}

fn win_enter_ext(wp_in: &mut WinInfoInfo, flags: i32) {
    let wp = crate::globals::WinPtr::from(wp_in);
    let mut other_buffer = false;
    let curwin_invalid = (flags & WEE_CURWIN_INVALID) != 0;

    if wp == curwin_ptr() && !curwin_invalid {
        return;
    }

    if !curwin_invalid {
        leaving_window(curwin_mut());
    }

    if !curwin_invalid && (flags & WEE_TRIGGER_LEAVE_AUTOCMDS) != 0 {
        if !ptr::eq(wp.borrow_mut().w_buffer.as_ptr(), g_st().curbuf.as_ptr()) {
            apply_autocmds(EVENT_BUFLEAVE, None, None, false, curbuf_opt_mut());
            other_buffer = true;
            if !win_valid(wp.opt()) {
                return;
            }
        }
        apply_autocmds(EVENT_WINLEAVE, None, None, false, curbuf_opt_mut());
        if !win_valid(wp.opt()) {
            return;
        }
        if aborting() {
            return;
        }
    }

    let wp = wp.borrow_mut();
    if (flags & WEE_UNDO_SYNC) != 0 && g_st().curbuf != wp.w_buffer {
        u_sync(false);
    }

    if opt_vars().p_spk.first_byte() == b'c' && !curwin_invalid {
        update_topline(curwin_mut());
    }

    if g_st().curbuf != wp.w_buffer {
        buf_copy_options((*wp).buf_mut(), BCO_ENTER | BCO_NOHELP);
    }
    if !curwin_invalid {
        g_st().prevwin = curwin_ptr();
        curwin_mut().w_redr_status = true;
    }
    set_curwin(&mut *wp);
    g_st().curbuf = wp.w_buffer;

    check_cursor(curwin_mut());
    if !virtual_active(curwin_mut()) {
        curwin_mut().w_cursor.coladd = 0;
    }
    if opt_vars().p_spk.first_byte() == b'c' {
        changed_line_abv_curs();
    } else {
        win_fix_cursor((get_real_state() & (MODE_NORMAL | MODE_CMDLINE | MODE_TERMINAL)) != 0);
    }

    update_cwd(K_CD_CAUSE_WINDOW);

    entering_window(curwin_mut());
    if (flags & WEE_TRIGGER_NEW_AUTOCMDS) != 0 {
        apply_autocmds(EVENT_WINNEW, None, None, false, curbuf_opt_mut());
    }
    if (flags & WEE_TRIGGER_ENTER_AUTOCMDS) != 0 {
        apply_autocmds(EVENT_WINENTER, None, None, false, curbuf_opt_mut());
        if other_buffer {
            apply_autocmds(EVENT_BUFENTER, None, None, false, curbuf_opt_mut());
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
        && crate::globals::prevwin_mut().w_hl_attr_normal
            != crate::globals::prevwin_mut().w_hl_attr_normalnc
    {
        redraw_later(prevwin_opt(), UPD_NOT_VALID);
    }

    if OptInt::from(curwin_mut().w_height) < opt_vars().p_wh
        && !curwin_mut().w_onebuf_opt.wo_wfh
        && !curwin_mut().w_floating
    {
        win_setheight(opt_vars().p_wh as i32);
    } else if curwin_mut().w_height == 0 {
        win_setheight(1);
    }

    if OptInt::from(curwin_mut().w_width) < opt_vars().p_wiw
        && !curwin_mut().w_onebuf_opt.wo_wfw
        && !curwin_mut().w_floating
    {
        win_setwidth(opt_vars().p_wiw as i32);
    }

    if !curwin_invalid
        && win_valid(g_st().prevwin.opt())
        && crate::globals::prevwin_mut().w_onebuf_opt.wo_cole > 0
        && !conceal_cursor_line(crate::globals::prevwin_mut())
    {
        conceal_redraw_cursor_line(crate::globals::prevwin_mut());
    }
    if curwin_mut().w_onebuf_opt.wo_cole > 0 && !conceal_cursor_line(curwin_mut()) {
        conceal_redraw_cursor_line(curwin_mut());
    }

    setmouse();
    do_autochdir();
}

pub fn update_cwd(cause: CdCause) {
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

    let mut cwd = [0u8; MAXPATHL];
    if !os_dirname(&mut cwd) {
        cwd[0] = 0;
    }

    if let Some(ref new_dir_cstr) = new_dir {
        if g_st().globaldir.is_none() && cwd[0] != 0 {
            g_st().globaldir = Some(crate::memory::cstring_until_nul(&cwd));
        }
        let dir_differs = !path_equal(
            new_dir_cstr,
            crate::memory::bytes_as_cstr(&cwd),
            K_PATH_CMP_LITERAL,
        );
        if !opt_vars().p_acd && dir_differs {
            do_autocmd_dirchanged(new_dir_cstr, scope, cause, true);
        }
        if os_chdir(new_dir_cstr) == 0 && !opt_vars().p_acd && dir_differs {
            do_autocmd_dirchanged(new_dir_cstr, scope, cause, false);
        }
        g_st().last_chdir_reason = None;
        shorten_fnames(true);
    } else if let Some(gdir) = g_st().globaldir.take() {
        let dir_differs = !path_equal(
            &gdir,
            crate::memory::bytes_as_cstr(&cwd),
            K_PATH_CMP_LITERAL,
        );
        if !opt_vars().p_acd && dir_differs {
            do_autocmd_dirchanged(&gdir, K_CD_SCOPE_GLOBAL, cause, true);
        }
        if os_chdir(&gdir) == 0 && !opt_vars().p_acd && dir_differs {
            do_autocmd_dirchanged(&gdir, K_CD_SCOPE_GLOBAL, cause, false);
        }
        g_st().last_chdir_reason = None;
        shorten_fnames(true);
    }
}

pub fn buf_jump_open_win(buf_in: &mut FileBuf) -> Option<&'static mut WinInfoInfo> {
    let buf = crate::globals::BufPtr::from(buf_in);
    if curwin_mut().w_buffer == buf {
        win_enter(curwin_mut(), false);
        return curwin_ptr().opt_mut();
    }
    for wp in for_all_windows_ptr() {
        if wp.w_buffer == buf {
            let wp_ptr = crate::globals::WinPtr::from(&*wp);
            win_enter(wp_ptr.borrow_mut(), false);
            return wp_ptr.opt_mut();
        }
    }
    None
}

pub fn buf_jump_open_tab(buf_in: &mut FileBuf) -> Option<&'static mut WinInfoInfo> {
    let buf = crate::globals::BufPtr::from(buf_in);
    if let Some(wp) = buf_jump_open_win(buf.borrow_mut()) {
        return Some(wp);
    }

    for tp in for_all_tabpages_ptr() {
        if g_st().curtab == tp {
            continue;
        }
        for wp2 in win_iter_ptr(tp.tp_firstwin.as_ptr()) {
            if wp2.w_buffer == buf {
                let wp2_ptr = crate::globals::WinPtr::from(&*wp2);
                goto_tabpage_win(&mut *tp, wp2_ptr.borrow_mut());
                return if curwin_ptr() == wp2_ptr {
                    wp2_ptr.opt_mut()
                } else {
                    None
                };
            }
        }
    }

    None
}

pub fn win_alloc(after_in: Option<&mut WinInfoInfo>, hidden: bool) -> &'static mut WinInfoInfo {
    let after = crate::globals::WinPtr::from(after_in);
    let new_wp = Box::leak(Box::new(WinInfoInfo::new()));

    WIN_STATE.with(|s| s.last_win_id.set(s.last_win_id.get() + (1)));
    new_wp.handle = WIN_STATE.with(|s| s.last_win_id.get());
    g_st().window_handles.insert(new_wp.handle, new_wp.into());

    new_wp.w_grid_alloc.target_kind = GridTarget::Win(new_wp.handle);
    grid_assign_handle(&mut new_wp.w_grid_alloc);

    new_wp.w_winvar = crate::eval::typval::tv_dict_item_alloc_static().di_id;
    init_var_dict(tv_dict_alloc(), new_wp.w_winvar.borrow_mut(), VAR_SCOPE);

    block_autocmds();
    if !hidden {
        let mut tp = crate::globals::TabpagePtr::NULL;
        if let Some(after_ref) = after.opt() {
            tp = crate::globals::TabpagePtr::from(win_find_tabpage(after_ref));
            if g_st().curtab == tp {
                tp = crate::globals::TabpagePtr::NULL;
            }
        }
        win_append(after.opt_mut(), &mut *new_wp, tp.opt_mut());
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

pub fn win_free(wp_in: &mut WinInfoInfo, tp: Option<&mut TabPageInfo>) {
    let wp = crate::globals::WinPtr::from(wp_in);
    {
        let wp = wp.borrow_mut();
        g_st().window_handles.remove(&wp.handle);
        clear_folding(&mut *wp);

        wp.w_alist = None;

        block_autocmds();

        wp.w_ns_set.clear();

        clear_winopt(&mut wp.w_onebuf_opt);
        clear_winopt(&mut wp.w_allbuf_opt);

        wp.w_p_lcs_chars.multispace = Vec::new();
        wp.w_p_lcs_chars.leadmultispace = Vec::new();

        vars_clear(&mut (*wp).w_vars().dv_hashtab);
        unref_var_dict(&mut *(*wp).w_vars());

        if g_st().prevwin == wp {
            g_st().prevwin = crate::globals::WinPtr::NULL;
        }
        for ttp in for_all_tabpages_mut() {
            if ttp.tp_prevwin == wp {
                ttp.tp_prevwin = crate::globals::WinPtr::NULL;
            }
        }

        wp.w_lines.clear();

        for i in 0..wp.w_tagstacklen as usize {
            tagstack_clear_entry(&mut wp.w_tagstack[i]);
        }
        wp.w_tagstacklen = 0;

        wp.w_localdir = None;
        wp.w_prevdir = None;

        stl_clear_click_defs(&mut wp.w_status_click_defs);
        stl_clear_click_defs(&mut wp.w_winbar_click_defs);

        for (_, lnum_click_defs) in std::mem::take(&mut wp.w_statuscol_click_defs) {
            for (_, mut row_click_defs) in lnum_click_defs {
                stl_clear_click_defs(&mut row_click_defs);
            }
        }

        for buf in for_all_buffers_mut() {
            let mut pos_wip = None;
            let mut pos_null = None;
            for (i, wip) in buf.b_wininfo.iter().enumerate() {
                if wip.wi_win == ptr::from_mut(wp) {
                    pos_wip = Some(i);
                } else if wip.wi_win.is_null() {
                    pos_null = Some(i);
                }
            }

            if let Some(pw) = pos_wip {
                let wip_wp = &mut (&mut buf.b_wininfo)[pw];
                wip_wp.wi_win = crate::globals::WinPtr::NULL;
                if wp.w_config.style == K_WIN_STYLE_MINIMAL && wip_wp.wi_optset {
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

        wp.w_config.title_chunks.clear();
        wp.w_config.footer_chunks.clear();

        clear_matches(&mut *wp);
        free_jumplist(&mut *wp);
        qf_free_all(Some(&mut *wp));

        wp.w_p_cc_cols.clear();

        win_free_grid(&mut *wp, false);

        if win_valid_any_tab(Some(&*wp)) {
            win_remove(&mut *wp, tp);
        }
    }
    if g_st().autocmd_busy {
        g_st().au_pending_free_win.push(wp.into_box());
    } else {
        wp.drop_box();
    }

    unblock_autocmds();
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
        if let Some(w_winvar) = self.w_winvar.opt_mut() {
            crate::eval::typval::tv_dict_item_free_slot(w_winvar);
        }
    }
}

pub fn win_free_grid(wp: &mut WinInfoInfo, reinit: bool) {
    if wp.w_grid_alloc.handle != 0 && ui_has(K_UIMULTIGRID) {
        ui_call_grid_destroy(Integer::from(wp.w_grid_alloc.handle));
    }
    grid_free(&mut wp.w_grid_alloc);
    if reinit {
        wp.w_grid_alloc = ScreenGrid::with_target(GridTarget::Win(wp.handle));
    }
}

pub fn win_append(
    after_in: Option<&mut WinInfoInfo>,
    wp_in: &mut WinInfoInfo,
    tp_in: Option<&mut TabPageInfo>,
) {
    let after = crate::globals::WinPtr::from(after_in);
    let wp = crate::globals::WinPtr::from(&*wp_in);
    let tp = crate::globals::TabpagePtr::from(tp_in);
    debug_assert!(tp.is_null() || g_st().curtab != tp);

    let first = if tp.is_null() {
        &mut g_st().firstwin
    } else {
        &mut tp.borrow_mut().tp_firstwin
    };
    let last = if tp.is_null() {
        &mut g_st().lastwin
    } else {
        &mut tp.borrow_mut().tp_lastwin
    };

    let mut before = if after.is_null() {
        *first
    } else {
        after.w_next
    };

    wp.borrow_mut().w_next = before;
    wp.borrow_mut().w_prev = after;

    if after.is_null() {
        *first = wp;
    } else {
        after.borrow_mut().w_next = wp;
    }
    if before.is_null() {
        *last = wp;
    } else {
        before.w_prev = wp;
    }
}

pub fn win_remove(wp: &mut WinInfoInfo, mut tp_opt: Option<&mut TabPageInfo>) {
    debug_assert!(tp_opt.as_deref().is_none_or(|t| g_st().curtab != t));

    if let Some(prev) = wp.prev_opt() {
        prev.w_next = wp.w_next;
    } else if let Some(ref mut tp_ref) = tp_opt {
        tp_ref.tp_firstwin = wp.w_next;
    } else {
        g_st().firstwin = (wp.w_next.as_ptr()).into();
        curtab_mut().tp_firstwin = wp.w_next;
    }
    if let Some(next) = wp.next_opt() {
        next.w_prev = wp.w_prev;
    } else if let Some(tp_ref) = tp_opt {
        tp_ref.tp_lastwin = wp.w_prev;
    } else {
        g_st().lastwin = (wp.w_prev.as_ptr()).into();
        curtab_mut().tp_lastwin = wp.w_prev;
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
    if WIN_STATE.with(|s| s.old_rows.get()) != g_st().rows_global {
        if opt_vars().p_window == OptInt::from(WIN_STATE.with(|s| s.old_rows.get()) - 1)
            || (WIN_STATE.with(|s| s.old_rows.get()) == 0 && !option_was_set(K_OPT_WINDOW))
        {
            opt_vars().p_window = OptInt::from(g_st().rows_global - 1);
        }
        WIN_STATE.with(|s| s.old_rows.set(g_st().rows_global));
        win_new_screen_rows();
    }
    if WIN_STATE.with(|s| s.old_columns.get()) != g_st().columns_global {
        WIN_STATE.with(|s| s.old_columns.set(g_st().columns_global));
        win_new_screen_cols();
    }
}

pub fn win_new_screen_rows() {
    if g_st().firstwin.is_null() {
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
    curtab_mut().tp_ch_used = opt_vars().p_ch;

    if !g_st().skip_win_fix_scroll {
        win_fix_scroll(true);
    }
}

pub fn win_new_screen_cols() {
    if g_st().firstwin.is_null() {
        return;
    }

    frame_new_width(&topframe_mut(), g_st().columns_global, false, true);
    if !frame_check_width(&topframe_mut(), g_st().columns_global) {
        frame_new_width(&topframe_mut(), g_st().columns_global, false, false);
    }

    win_comp_pos();
    win_reconfig_floats();
}

pub fn snapshot_windows_scroll_size() {
    for wp in for_all_windows_mut() {
        wp.w_last_topline = wp.w_topline;
        wp.w_last_topfill = wp.w_topfill;
        wp.w_last_leftcol = wp.w_leftcol;
        wp.w_last_skipcol = wp.w_skipcol;
        wp.w_last_width = wp.w_width;
        wp.w_last_height = wp.w_height;
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
) -> Option<&'static mut TvDict> {
    let d = tv_dict_alloc();
    d.dv_refcount = 1;

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
        if !tv_dict_add_tv(&mut *d, key.to_bytes(), &mut tv) {
            tv_dict_unref(Some(d));
            return None;
        }
    }
    Some(d)
}

fn check_window_scroll_resize(
    mut size_count: Option<&mut i32>,
    first_scroll_win: &mut Option<&'static mut WinInfoInfo>,
    first_size_win: &mut Option<&'static mut WinInfoInfo>,
    mut winlist: Option<&mut TvList>,
    mut v_event: Option<&mut TvDict>,
) {
    let mut tot_width: i32 = 0;
    let mut tot_height: i32 = 0;
    let mut tot_topline: i32 = 0;
    let mut tot_topfill: i32 = 0;
    let mut tot_leftcol: i32 = 0;
    let mut tot_skipcol: i32 = 0;

    for wp in for_all_windows_ptr() {
        if wp.w_floating && wp.w_last_topline == 0 {
            wp.w_last_topline = wp.w_topline;
            wp.w_last_topfill = wp.w_topfill;
            wp.w_last_leftcol = wp.w_leftcol;
            wp.w_last_skipcol = wp.w_skipcol;
            wp.w_last_width = wp.w_width;
            wp.w_last_height = wp.w_height;
            continue;
        }

        let eiw = wp.w_onebuf_opt.wo_eiw.as_cstr();
        let ignore_scroll = event_ignored(EVENT_WINSCROLLED, eiw);
        let size_changed = !event_ignored(EVENT_WINRESIZED, eiw)
            && (wp.w_last_width != wp.w_width || wp.w_last_height != wp.w_height);
        if size_changed {
            if let Some(ref mut wl) = winlist {
                let tv = TypVal {
                    v_type: VarType::VarNumber,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(i64::from(wp.handle)),
                };
                tv_list_append_owned_tv(wl, tv);
            } else if let Some(ref mut size_count) = size_count {
                **size_count += 1;
                if first_size_win.is_none() {
                    *first_size_win = Some(crate::globals::WinPtr::from(&*wp).borrow_mut());
                }
                if first_scroll_win.is_none() && !ignore_scroll {
                    *first_scroll_win = Some(crate::globals::WinPtr::from(&*wp).borrow_mut());
                }
            }
        }

        let scroll_changed = !ignore_scroll
            && (wp.w_last_topline != wp.w_topline
                || wp.w_last_topfill != wp.w_topfill
                || wp.w_last_leftcol != wp.w_leftcol
                || wp.w_last_skipcol != wp.w_skipcol);
        if scroll_changed && first_scroll_win.is_none() {
            *first_scroll_win = Some(crate::globals::WinPtr::from(&*wp).borrow_mut());
        }

        if (size_changed || scroll_changed)
            && let Some(ref mut ve) = v_event
        {
            let width = wp.w_width - wp.w_last_width;
            let height = wp.w_height - wp.w_last_height;
            let topline = wp.w_topline - wp.w_last_topline;
            let topfill = wp.w_topfill - wp.w_last_topfill;
            let leftcol = wp.w_leftcol - wp.w_last_leftcol;
            let skipcol = wp.w_skipcol - wp.w_last_skipcol;
            let Some(d) = make_win_info_dict(width, height, topline, topfill, leftcol, skipcol)
            else {
                break;
            };
            let mut winid = [0u8; NUMBUFLEN];
            let key_len = format_to_buf!(&mut winid[..], c"%d", wp.handle);
            let _ = key_len;
            let mut d_ptr = crate::eval::typval::DictId::from(&*d);
            if !tv_dict_add_dict(
                ve,
                crate::memory::bytes_as_cstr(&winid),
                Some(d_ptr.borrow_mut()),
            ) {
                tv_dict_unref(Some(d_ptr.borrow_mut()));
                break;
            }
            d_ptr.dv_refcount -= 1;

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
        let mut alldict_ptr = crate::eval::typval::DictId::from(&*alldict);
        if !tv_dict_add_dict(v_event, c"all", Some(alldict_ptr.borrow_mut())) {
            tv_dict_unref(Some(alldict_ptr.borrow_mut()));
        } else {
            alldict_ptr.dv_refcount -= 1;
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
    let mut first_scroll_win: Option<&'static mut WinInfoInfo> = None;
    let mut first_size_win: Option<&'static mut WinInfoInfo> = None;
    let mut dummy_scroll: Option<&'static mut WinInfoInfo> = None;
    let mut dummy_size: Option<&'static mut WinInfoInfo> = None;
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

    let mut windows_list: Option<&'static mut TvList> = None;
    if trigger_resize {
        let wl = tv_list_alloc(size_count as isize);
        check_window_scroll_resize(
            None,
            &mut dummy_scroll,
            &mut dummy_size,
            Some(&mut *wl),
            None,
        );
        windows_list = Some(wl);
    }

    let mut scroll_dict: Option<&'static mut TvDict> = None;
    if trigger_scroll {
        let sd = tv_dict_alloc();
        sd.dv_refcount = 1;
        check_window_scroll_resize(
            None,
            &mut dummy_scroll,
            &mut dummy_size,
            None,
            Some(&mut *sd),
        );
        scroll_dict = Some(sd);
    }

    snapshot_windows_scroll_size();

    WIN_STATE.with(|s| s.win_scrolled_resized_recursive.set(true));

    let mut resize_winid = [0u8; NUMBUFLEN];
    let mut resize_win = crate::globals::WinPtr::NULL;
    let mut resize_bufref: Bufref = Bufref::new();
    if trigger_resize && let Some(rw) = first_size_win {
        resize_win = crate::globals::WinPtr::from(&*rw);
        format_to_buf!(&mut resize_winid[..], c"%d", rw.handle);
        set_bufref(&mut resize_bufref, rw.w_buffer.opt_mut());
    }

    let mut scroll_winid = [0u8; NUMBUFLEN];
    let mut scroll_win = crate::globals::WinPtr::NULL;
    let mut scroll_bufref: Bufref = Bufref::new();
    if trigger_scroll && let Some(sw) = first_scroll_win {
        scroll_win = crate::globals::WinPtr::from(&*sw);
        format_to_buf!(&mut scroll_winid[..], c"%d", sw.handle);
        set_bufref(&mut scroll_bufref, sw.w_buffer.opt_mut());
    }

    if let Some(windows_list) = windows_list {
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let v_event = get_v_event(&mut save_v_event);

        if tv_dict_add_list(&mut *v_event, c"windows", Some(windows_list)) {
            tv_dict_set_keys_readonly(&mut *v_event);
            let buf = if bufref_valid(Some(&resize_bufref)) {
                resize_bufref.br_buf
            } else {
                g_st().curbuf
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
        restore_v_event(&mut *v_event, &mut save_v_event);
    }

    if let Some(scroll_dict) = scroll_dict {
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let v_event = get_v_event(&mut save_v_event);

        tv_dict_extend(&mut *v_event, &mut *scroll_dict, b"move");
        tv_dict_set_keys_readonly(&mut *v_event);
        tv_dict_unref(Some(scroll_dict));

        let buf = if bufref_valid(Some(&scroll_bufref)) {
            scroll_bufref.br_buf
        } else {
            g_st().curbuf
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

        restore_v_event(&mut *v_event, &mut save_v_event);
    }

    WIN_STATE.with(|s| s.win_scrolled_resized_recursive.set(false));
}

pub fn win_size_save(gap: &mut Vec<i32>) {
    gap.clear();
    gap.reserve((win_count() * 2 + 1) as usize);
    gap.push(rows_avail() as i32 + global_stl_height() - last_stl_height(false));

    for wp in for_all_windows_mut() {
        gap.push(wp.w_width + wp.w_vsep_width);
        gap.push(wp.w_height);
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
                if !wp.w_floating {
                    frame_setwidth(&wp.frame_mut(), width, true);
                    win_setheight_win(height, wp, true);
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

    for wp in for_all_windows_rev_ptr().take_while(|wp| wp.w_floating) {
        if wp.w_config.relative == K_FLOAT_RELATIVE_WINDOW {
            wp.w_pos_changed = true;
        }
    }

    row + global_stl_height()
}

fn frame_comp_pos(topfrp: &Frame, row: &mut i32, col: &mut i32) {
    if let Some(wp) = topfrp.win_opt() {
        if wp.w_winrow != *row || wp.w_wincol != *col {
            wp.w_winrow = *row;
            wp.w_wincol = *col;
            redraw_later(Some(wp), UPD_NOT_VALID);
            wp.w_redr_status = true;
            wp.w_pos_changed = true;
        }
        let h = wp.w_height + wp.w_hsep_height + wp.w_status_height;
        *row += if h > topfrp.fr_height.get() {
            topfrp.fr_height.get()
        } else {
            h
        };
        *col += wp.w_width + wp.w_vsep_width;
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
    win_setheight_win(height, crate::globals::curwin_mut(), true);
}

pub fn win_setheight_win(mut height: i32, win: &mut WinInfoInfo, from_top: bool) {
    let min_wmh = if win == curwin_ptr() {
        opt_vars().p_wmh.max(1)
    } else {
        opt_vars().p_wmh
    } as i32;
    height = height.max(min_wmh + win.w_winbar_height);

    if win.w_floating {
        win.w_config.height = height.max(1);
        win_config_float(win, win.w_config.clone());
        redraw_later(Some(&mut *win), UPD_VALID);
    } else {
        frame_setheight(
            &win.frame_mut(),
            height + win.w_hsep_height + win.w_status_height,
            from_top,
        );

        win_comp_pos();
        win_fix_scroll(true);

        redraw_all_later(UPD_NOT_VALID);
        g_st().redraw_cmdline = true;
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
                    && frp.win_mut().w_onebuf_opt.wo_wfh
                {
                    room_reserved += frp.fr_height.get();
                }
                room += frp.fr_height.get();
                if !ptr::eq(frp.borrow(), curfrp) {
                    room -= frame_minheight(&frp, crate::globals::WinPtr::NULL);
                }
            }
            if !from_top || curfrp.fr_width.get() != g_st().columns_global {
                room_cmdline = 0;
            } else {
                let wp = lastwin_nofloating(None);
                room_cmdline = g_st().rows_global
                    - opt_vars().p_ch as i32
                    - global_stl_height()
                    - (wp.w_winrow + wp.w_height + wp.w_hsep_height + wp.w_status_height);
                room_cmdline = room_cmdline.max(0);
            }

            if height <= room + room_cmdline {
                break;
            }
            if run == 2 || curfrp.fr_width.get() == g_st().columns_global {
                height = room + room_cmdline;
                break;
            }

            frame_setheight(
                curfrp.fr_parent.get().borrow(),
                height + frame_minheight(curfrp.fr_parent.get().borrow(), nowin())
                    - opt_vars().p_wmh as i32
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
                    && frp.fr_win.get().w_onebuf_opt.wo_wfh
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
    win_setwidth_win(width, crate::globals::curwin_mut(), true);
}

pub fn win_setwidth_win(mut width: i32, wp: &mut WinInfoInfo, from_left: bool) {
    if wp == curwin_ptr() {
        width = width.max(opt_vars().p_wmw as i32).max(1);
    } else if width < 0 {
        width = 0;
    }
    if wp.w_floating {
        wp.w_config.width = width;
        win_config_float(wp, wp.w_config.clone());
        redraw_later(Some(&mut *wp), UPD_NOT_VALID);
    } else {
        frame_setwidth(&wp.frame_mut(), width + wp.w_vsep_width, from_left);

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
                    && frp.win_mut().w_onebuf_opt.wo_wfw
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
                    - opt_vars().p_wmw as i32
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
                    && frp.fr_win.get().w_onebuf_opt.wo_wfw
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
    while opt_vars().p_wmh > 0 {
        let room = g_st().rows_global - opt_vars().p_ch as i32;
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
    None
}

pub fn did_set_winminwidth(_args: &mut Optset) -> Option<&'static CStr> {
    let mut first = true;
    while opt_vars().p_wmw > 0 {
        let room = g_st().columns_global;
        let needed = frame_minwidth(&topframe_mut(), crate::globals::WinPtr::NULL);
        if room >= needed {
            break;
        }
        opt_vars().p_wmw -= 1;
        if first {
            emsg(gettext(E_NOROOM));
            first = false;
        }
    }
    None
}

pub fn win_drag_status_line(dragwin: &mut WinInfoInfo, mut offset: i32) {
    let mut fr = dragwin.w_frame.clone();
    let mut curfr = fr.clone();
    if g_st().topframe != fr {
        fr = fr.fr_parent.get();

        if fr.fr_layout.get() != FR_COL {
            curfr = fr.clone();
            if g_st().topframe != fr {
                fr = fr.fr_parent.get();
            }
        }
    }

    while g_st().topframe != curfr && curfr.fr_next.get().is_null() {
        if g_st().topframe != fr {
            fr = fr.fr_parent.get();
        }
        curfr = fr.clone();
        if g_st().topframe != fr {
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
        room = g_st().rows_global - g_st().cmdline_row;
        if !curfr.fr_next.get().is_null() {
            room -= opt_vars().p_ch as i32 + global_stl_height();
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

pub fn win_drag_vsep_line(dragwin: &mut WinInfoInfo, mut offset: i32) {
    let mut fr = dragwin.w_frame.clone();
    if g_st().topframe == fr {
        return;
    }
    let mut curfr = fr.clone();

    fr = fr.fr_parent.get();

    if fr.fr_layout.get() != FR_ROW {
        if g_st().topframe == fr {
            return;
        }
        curfr = fr.clone();

        fr = fr.fr_parent.get();
    }

    while curfr.fr_next.get().is_null() {
        if g_st().topframe == fr {
            break;
        }
        curfr = fr.clone();

        fr = fr.fr_parent.get();

        if g_st().topframe != fr {
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

pub fn set_fraction(wp: &mut WinInfoInfo) {
    if wp.w_view_height > 1 {
        wp.w_fraction = (wp.w_wrow * FRACTION_MULT + FRACTION_MULT / 2) / wp.w_view_height;
    }
}

pub fn win_fix_scroll(resize: bool) {
    if opt_vars().p_spk.first_byte() == b'c' {
        return;
    }

    g_st().skip_update_topline = true;
    for wp in for_all_windows_ptr() {
        if !wp.w_floating && wp.w_height != wp.w_prev_height {
            wp.w_do_win_fix_cursor = true;

            if opt_vars().p_spk.first_byte() == b's'
                && wp.w_winrow != wp.w_prev_winrow
                && wp.w_botline - 1 <= (*wp).buf_mut().b_ml.ml_line_count
            {
                let diff = (wp.w_winrow - wp.w_prev_winrow) + (wp.w_height - wp.w_prev_height);
                let cursor = wp.w_cursor;
                wp.w_cursor.lnum = wp.w_botline - 1;

                if diff > 0 {
                    cursor_down_inner(&mut *wp, diff, false);
                } else {
                    cursor_up_inner(&mut *wp, -diff, false);
                }

                wp.w_fraction = FRACTION_MULT;
                scroll_to_fraction(wp, wp.w_prev_height);
                wp.w_cursor = cursor;
                wp.w_valid &= !VALID_WCOL;
            } else if wp == curwin_ptr() {
                wp.w_valid &= !VALID_CROW;
            }

            invalidate_botline_win(&mut *wp);
            validate_botline_win(&mut *wp);
        }
        wp.w_prev_height = wp.w_height;
        wp.w_prev_winrow = wp.w_winrow;
    }
    g_st().skip_update_topline = false;
    if (get_real_state() & (MODE_NORMAL | MODE_CMDLINE | MODE_TERMINAL)) == 0 {
        win_fix_cursor(false);
    } else if resize {
        win_fix_cursor(true);
    }
}

fn win_fix_cursor(normal: bool) {
    let wp = curwin_mut();
    let wp = &mut *wp;

    if !wp.w_do_win_fix_cursor || (*wp).buf_mut().b_ml.ml_line_count < wp.w_view_height {
        return;
    }

    wp.w_do_win_fix_cursor = false;
    let so = (OptInt::from(wp.w_view_height / 2).min(get_scrolloff_value(&mut *wp))) as i32;
    let lnum = wp.w_cursor.lnum;

    wp.w_cursor.lnum = wp.w_topline;
    cursor_down_inner(&mut *wp, so, false);
    let top = wp.w_cursor.lnum;

    wp.w_cursor.lnum = wp.w_botline - 1;
    cursor_up_inner(&mut *wp, so, false);
    let bot = wp.w_cursor.lnum;

    wp.w_cursor.lnum = lnum;
    let mut nlnum: Linenr = 0;
    if lnum > bot && (wp.w_botline - (*wp).buf_mut().b_ml.ml_line_count) != 1 {
        nlnum = bot;
    } else if lnum < top && wp.w_topline != 1 {
        nlnum = if so == wp.w_view_height / 2 { bot } else { top };
    }

    if nlnum != 0 {
        if normal {
            setmark(b'\'' as i32);
            wp.w_cursor.lnum = nlnum;
        } else {
            wp.w_fraction = if nlnum == bot { FRACTION_MULT } else { 0 };
            scroll_to_fraction(wp, wp.w_prev_height);
            validate_botline_win(curwin_mut());
        }
    }
}

pub fn win_new_height(wp: &mut WinInfoInfo, mut height: i32) {
    height = height.max(0);
    if wp.w_height == height {
        return;
    }

    wp.w_height = height;
    wp.w_pos_changed = true;
    win_set_inner_size(wp, true);
}

pub fn scroll_to_fraction(wp: &mut WinInfoInfo, prev_height: i32) {
    let height = wp.w_view_height;

    if height > 0
        && (!wp.w_onebuf_opt.wo_scb || wp == curwin_ptr())
        && (height < wp.buf_mut().b_ml.ml_line_count || wp.w_topline > 1)
    {
        let mut lnum = wp.w_cursor.lnum.max(1);
        wp.w_wrow = (wp.w_fraction * height - 1) / FRACTION_MULT;
        let mut line_size = plines_win_col(wp, lnum, wp.w_cursor.col as i64) - 1;
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
                has_folding(wp, lnum, Some(&mut lnum), None);
                if lnum == 1 {
                    line_size = i32::from(!decor_conceal_line(wp, lnum - 1, false));
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
                has_folding(wp, lnum, None, Some(&mut lnum));
                lnum += 1;
                wp.w_wrow -= line_size + sline;
            } else if sline > 0 {
                lnum = 1;
                wp.w_wrow -= sline;
            }
        }
        set_topline(wp, lnum);
    }

    if wp == curwin_ptr() {
        curs_columns(&mut *wp, false);
    }
    if prev_height > 0 {
        wp.w_prev_fraction_row = wp.w_wrow;
    }

    redraw_later(Some(&mut *wp), UPD_SOME_VALID);
    invalidate_botline_win(wp);
}

pub fn win_set_inner_size(wp: &mut WinInfoInfo, valid_cursor: bool) {
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
            if wp == curwin_ptr() && (opt_vars().p_spk.first_byte() == b'c' || wp.w_floating) {
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
            && (opt_vars().p_spk.first_byte() == b'c' || wp.w_floating)
        {
            if !wp.w_onebuf_opt.wo_sms {
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
            if wp == curwin_ptr() && (opt_vars().p_spk.first_byte() == b'c' || wp.w_floating) {
                curs_columns(&mut *wp, true);
            }
        }
        redraw_later(Some(&mut *wp), UPD_NOT_VALID);
    }

    if let Some(term) = wp.buf_mut().terminal_opt() {
        terminal_check_size(term);
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

pub fn win_new_width(wp: &mut WinInfoInfo, width: i32) {
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
    let mut old_p_ch = curtab_mut().tp_ch_used as i32;

    let mut frp = lastwin_nofloating(None).w_frame.clone();
    while frp.fr_width.get() != g_st().columns_global && !frp.fr_parent.get().is_null() {
        frp = frp.fr_parent.get();
    }

    while !frp.fr_prev.get().is_null()
        && frp.fr_layout.get() == FR_LEAF
        && frp.fr_win.get().w_onebuf_opt.wo_wfh
    {
        frp = frp.fr_prev.get();
    }

    while opt_vars().p_ch > OptInt::from(old_p_ch)
        && WIN_STATE.with(|s| s.command_frame_height.get())
    {
        if frp.is_null() {
            emsg(gettext(E_NOROOM));
            opt_vars().p_ch = OptInt::from(old_p_ch);
            break;
        }
        let h = ((opt_vars().p_ch - OptInt::from(old_p_ch)) as i32)
            .min(frp.fr_height.get() - frame_minheight(frp.borrow(), crate::globals::WinPtr::NULL));
        frame_add_height(frp.borrow(), -h);
        old_p_ch += h;

        frp = frp.fr_prev.get();
    }
    if opt_vars().p_ch < OptInt::from(old_p_ch)
        && WIN_STATE.with(|s| s.command_frame_height.get())
        && !frp.is_null()
    {
        frame_add_height(
            frp.borrow(),
            (OptInt::from(old_p_ch) - opt_vars().p_ch) as i32,
        );
    }

    win_comp_pos();
    win_fix_scroll(true);
    g_st().cmdline_row = g_st().rows_global - opt_vars().p_ch as i32;
    g_st().redraw_cmdline = true;

    if g_st().msg_scrolled == 0 && g_st().full_screen {
        let grid = if !ui_has(K_UIMESSAGES) {
            msg_grid_validate();
            &mut g_st().msg_grid_adj
        } else {
            &mut g_st().default_gridview
        };
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
    WIN_STATE.with(|s| s.min_set_ch.set(opt_vars().p_ch));
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

fn find_horizontally_resizable_frame(fr: &Frame) -> Option<crate::globals::FramePtr> {
    let mut fp = crate::globals::FramePtr::from(fr);
    while fp.fr_height.get() <= frame_minheight(&fp, crate::globals::WinPtr::NULL) {
        if g_st().topframe == fp {
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
        win_new_height(wp, wp.w_height - 1);
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
        let is_last = is_bottom_win(wp);

        if is_last {
            if wp.w_status_height != 0 && (!statusline || is_stl_global) {
                win_remove_status_line(wp, false);
            } else if wp.w_status_height == 0 && !is_stl_global && statusline {
                wp.w_status_height = STATUS_HEIGHT;
                if !resize_frame_for_status(fr) {
                    return;
                }
                comp_col();
            }
            if (wp.w_height - wp.w_prev_height).abs() == 1 {
                wp.w_prev_height = wp.w_height;
            }
        } else if wp.w_status_height != 0 && is_stl_global {
            win_remove_status_line(wp, true);
        } else if wp.w_status_height == 0 && !is_stl_global {
            wp.w_status_height = STATUS_HEIGHT;
            wp.w_hsep_height = 0;
            comp_col();
        }
    } else {
        for fp in frame_iter_mut(fr.fr_child.get()) {
            last_status_rec(&fp, statusline, is_stl_global);
        }
    }
}

pub fn set_winbar_win(wp: &mut WinInfoInfo, make_room: bool, valid_cursor: bool) -> Option<bool> {
    let winbar_height = if wp.w_floating {
        i32::from(!wp.w_onebuf_opt.wo_wbr.is_empty())
    } else {
        i32::from(!opt_vars().p_wbr.is_empty() || !wp.w_onebuf_opt.wo_wbr.is_empty())
    };

    if wp.w_winbar_height != winbar_height {
        if winbar_height == 1 && wp.w_view_height <= 1 {
            if wp.w_floating {
                emsg(gettext(E_NOROOM));
                return None;
            } else if !make_room || !resize_frame_for_winbar(&wp.frame_mut()) {
                return Some(false);
            }
        }
        wp.w_winbar_height = winbar_height;
        win_set_inner_size(wp, valid_cursor);

        if winbar_height == 0 {
            stl_clear_click_defs(&mut wp.w_winbar_click_defs);
            wp.w_winbar_click_defs = Vec::new();
        }
    }

    Some(true)
}

fn set_winbar_curtab(make_room: bool, valid_cursor: bool) -> bool {
    for wp in for_all_windows_ptr() {
        if set_winbar_win(&mut *wp, make_room, valid_cursor) == Some(false) {
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
        if g_st().curtab != tp {
            let mut switchwin: CtxSwitch = CtxSwitch::new();
            if ctx_switch(
                &mut switchwin,
                tp.tp_curwin.opt_mut(),
                Some(tp),
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
    debug_assert!(!g_st().first_tabpage.is_null());
    match opt_vars().p_stal {
        0 => 0,
        1 => i32::from(!crate::globals::first_tabpage_mut().tp_next.is_null()),
        _ => 1,
    }
}

pub fn global_winbar_height() -> i32 {
    i32::from(!opt_vars().p_wbr.is_empty())
}

pub fn global_stl_height() -> i32 {
    if opt_vars().p_ls == 3 {
        STATUS_HEIGHT
    } else {
        0
    }
}

pub fn last_stl_height(morewin: bool) -> i32 {
    if opt_vars().p_ls > 1
        || (opt_vars().p_ls == 1 && (morewin || !one_window(firstwin_mut(), None)))
    {
        STATUS_HEIGHT
    } else {
        0
    }
}

pub fn min_rows(tp: &mut TabPageInfo) -> i32 {
    if g_st().firstwin.is_null() {
        return MIN_LINES;
    }

    let mut total = frame_minheight(&tp.topframe_mut(), crate::globals::WinPtr::NULL);
    total += tabline_height() + global_stl_height();
    if (if g_st().curtab == tp {
        opt_vars().p_ch
    } else {
        tp.tp_ch_used
    }) > 0
    {
        total += 1;
    }
    total
}

pub fn min_rows_for_all_tabpages() -> i32 {
    if g_st().firstwin.is_null() {
        return MIN_LINES;
    }

    let mut total: i32 = 0;
    for tp in for_all_tabpages_ptr() {
        let mut n = frame_minheight(&tp.topframe_mut(), crate::globals::WinPtr::NULL);
        if (if g_st().curtab == tp {
            opt_vars().p_ch
        } else {
            tp.tp_ch_used
        }) > 0
        {
            n += 1;
        }
        total = total.max(n);
    }
    total += tabline_height() + global_stl_height();
    total
}

pub fn only_one_window() -> bool {
    if !crate::globals::first_tabpage_mut().tp_next.is_null() {
        return false;
    }

    let mut count: i32 = 0;
    for wp in for_all_windows_ptr() {
        if !wp.w_buffer.is_null()
            && (!((bt_help(wp.buf_opt()) && !bt_help(Some(crate::globals::curbuf_mut())))
                || wp.w_floating
                || wp.w_onebuf_opt.wo_pvw)
                || wp == curwin_ptr())
            && !is_ctx_win(&*wp)
        {
            count += 1;
        }
    }
    count <= 1
}

fn check_lnums_both(do_curwin: bool, nested: bool) {
    for tp in for_all_tabpages_ptr() {
        for wp in win_iter_ptr(if g_st().curtab == tp {
            g_st().firstwin.as_ptr()
        } else {
            tp.tp_firstwin.as_ptr()
        }) {
            if (do_curwin || wp != curwin_ptr()) && g_st().curbuf == wp.w_buffer {
                if !nested {
                    wp.w_save_cursor.w_cursor_save = wp.w_cursor;
                    wp.w_save_cursor.w_topline_save = wp.w_topline;
                }

                let mut need_adjust = wp.w_cursor.lnum > curbuf_mut().b_ml.ml_line_count;
                if need_adjust {
                    wp.w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
                }
                if need_adjust || !nested {
                    wp.w_save_cursor.w_cursor_corr = wp.w_cursor;
                }

                need_adjust = wp.w_topline > curbuf_mut().b_ml.ml_line_count;
                if need_adjust {
                    wp.w_topline = curbuf_mut().b_ml.ml_line_count;
                }
                if need_adjust || !nested {
                    wp.w_save_cursor.w_topline_corr = wp.w_topline;
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
        for wp in win_iter_mut(if g_st().curtab == tp {
            g_st().firstwin.as_ptr()
        } else {
            tp.tp_firstwin.as_ptr()
        }) {
            if g_st().curbuf == wp.w_buffer {
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

pub fn make_snapshot(idx: i32) {
    curtab_mut().tp_snapshot[idx as usize] = Some(make_snapshot_rec(&topframe_mut()));
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

fn clear_snapshot(tp: &mut TabPageInfo, idx: i32) {
    tp.tp_snapshot[idx as usize] = None;
}

fn get_snapshot_curwin_rec(ft: &SnapshotFrame) -> Option<&'static mut WinInfoInfo> {
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
    ft.fr_win.opt_mut()
}

fn get_snapshot_curwin(idx: i32) -> Option<&'static mut WinInfoInfo> {
    let sn = curtab_mut().tp_snapshot[idx as usize].as_ref()?;
    get_snapshot_curwin_rec(sn)
}

pub fn restore_snapshot(idx: i32, close_curwin: i32) {
    if let Some(sn) = curtab_mut().tp_snapshot[idx as usize].take()
        && sn.fr_width == crate::globals::topframe_mut().fr_width.get()
        && sn.fr_height == crate::globals::topframe_mut().fr_height.get()
        && check_snapshot_rec(&sn, &topframe_mut())
    {
        let wp = restore_snapshot_rec(&sn, &topframe_mut());
        win_comp_pos();
        if let Some(wp_ref) = wp
            && close_curwin != 0
        {
            win_goto(wp_ref);
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

fn restore_snapshot_rec(sn: &SnapshotFrame, fr: &Frame) -> Option<&'static mut WinInfoInfo> {
    let mut wp = crate::globals::WinPtr::NULL;

    fr.fr_height.set(sn.fr_height);
    fr.fr_width.set(sn.fr_width);
    if fr.fr_layout.get() == FR_LEAF {
        let fr_h = fr.fr_height.get();
        frame_new_height(fr, fr_h, false, false, false);
        frame_new_width(fr, fr.fr_width.get(), false, false);
        wp = sn.fr_win;
    }
    if let Some(ref next) = sn.fr_next
        && let Some(wp2) = restore_snapshot_rec(next, &fr.next_mut())
    {
        wp = crate::globals::WinPtr::from(&*wp2);
    }
    if let Some(ref child) = sn.fr_child
        && let Some(wp2) = restore_snapshot_rec(child, &fr.child_mut())
    {
        wp = crate::globals::WinPtr::from(&*wp2);
    }
    wp.opt_mut()
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

pub fn check_colorcolumn(
    cc: Option<&CStr>,
    mut wp: Option<&mut WinInfoInfo>,
) -> Option<&'static CStr> {
    if wp.is_some() && wp.as_deref_mut().unwrap().w_buffer.is_null() {
        return None;
    }

    let cc_owned = if cc.is_none()
        && let Some(ref w) = wp
    {
        Some(w.w_onebuf_opt.wo_cc.to_owned())
    } else {
        None
    };
    let mut rem: &[u8] = cc.or(cc_owned.as_deref()).map_or(&[], CStr::to_bytes);

    let tw: OptInt = if wp.is_some() {
        wp.as_deref_mut().unwrap().buf_mut().b_p_tw
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
        wp.w_p_cc_cols.clear();
        if count > 0 {
            color_cols[..count].sort_unstable();
            wp.w_p_cc_cols.extend_from_slice(&color_cols[..count]);
            wp.w_p_cc_cols.dedup();
            wp.w_p_cc_cols.push(-1);
        }
    }

    None
}

pub fn win_locked(wp: &mut WinInfoInfo) -> i32 {
    wp.w_locked
}

pub fn win_get_tabwin(id: ObjHandle, tabnr: &mut i32, winnr: &mut i32) {
    *tabnr = 0;
    *winnr = 0;

    for (tnum, tp) in (1_i32..).zip(for_all_tabpages_ptr()) {
        let mut wnum: i32 = 1;
        for wp in for_all_windows_in_tab_mut(&*tp) {
            if wp.handle == id {
                if win_has_winnr(wp, &mut *tp) {
                    *winnr = wnum;
                    *tabnr = tnum;
                }
                return;
            }
            wnum += i32::from(win_has_winnr(wp, &mut *tp));
        }
    }
}

pub fn win_ui_flush(validate: bool) {
    for tp in for_all_tabpages_ptr() {
        for wp in win_iter_mut(if g_st().curtab == tp {
            g_st().firstwin.as_ptr()
        } else {
            tp.tp_firstwin.as_ptr()
        }) {
            if (wp.w_pos_changed || wp.w_grid_alloc.pending_comp_index_update)
                && !wp.w_grid_alloc.chars.is_empty()
            {
                if g_st().curtab == tp {
                    ui_ext_win_position(wp, validate);
                } else {
                    ui_call_win_hide(Integer::from(wp.w_grid_alloc.handle));
                    wp.w_pos_changed = false;
                }
                wp.w_grid_alloc.pending_comp_index_update = false;
            }
            if g_st().curtab == tp {
                ui_ext_win_viewport(wp);
            }
        }
    }
    pum_pos_ui_flush(false);
    msg_ui_flush();
}

pub fn lastwin_nofloating(tp: Option<&TabPageInfo>) -> &'static mut WinInfoInfo {
    debug_assert!(tp.is_none_or(|t| g_st().curtab != t));
    let mut res = if let Some(tp_ref) = tp {
        tp_ref.lastwin_mut()
    } else {
        lastwin_mut()
    };
    while res.w_floating {
        res = res.prev_mut();
    }
    res
}

pub fn win_is_focusable(wp: &WinInfoInfo) -> bool {
    wp.w_config.focusable && !wp.w_config.hide
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
    pub fn win_mut(&self) -> &'static mut WinInfoInfo {
        self.fr_win.get().borrow_mut()
    }

    #[inline]
    pub fn win_opt(&self) -> Option<&'static mut WinInfoInfo> {
        self.fr_win.get().opt_mut()
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
