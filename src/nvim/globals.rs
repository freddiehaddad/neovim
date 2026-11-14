#![deny(unsafe_code)]

//! Global variables (`src/nvim/globals.h`).

pub use crate::api::private::validate::CloneCell;
pub use crate::cursor::CursorPosCellExt;
use crate::highlight::{HLF_COUNT, RgbValue};

use crate::decoration::{DecorState, NS};
use crate::ex_docmd::Cmdmod;

use crate::api::ui::NO_SCREEN;
use crate::buffer::{Bufref, FileBuf, MAXLNUM, SEA_NONE};
use crate::context::CtxWin;
use crate::cursor::{Colnr, CursorPos, Linenr};
use crate::drawscreen::WinExtmark;
use crate::eval::CallerScope;
use crate::eval::typval::Callback;
use crate::event::multiqueue::MultiQueue;
use crate::ex_eval::{Except, Msglist};
use crate::ex_getln::{Disptick, Hlf};
use crate::extmark::TriState;
use crate::grid::{GridTarget, GridView, Sattr, Schar, ScreenGrid};
use crate::lua::converter::NluaRefState;
use crate::mapping::Typebuf;
use crate::mark::Xfmark;
use crate::r#match::{SearchState, SynMatch};
use crate::menu::Vimmenu;
use crate::mouse::StlClickDefinition;
use crate::msgpack_rpc::packer::ObjHandle;
use crate::normal::VisualState;
use crate::os::fs::NvimStats;
use crate::popupmenu::PumWant;
use crate::regexp::RegExtmatch;
use crate::runtime::Sctx;
use crate::state::{MODE_MAX_LENGTH, MODE_NORMAL};
use crate::textformat::InsState;
use crate::window::{Frame, TabPageInfo, WinInfoInfo};

pub(crate) const IOSIZE: usize = 1024 + 1;
pub(crate) const DFLT_ROWS: i32 = 24;
pub(crate) const DFLT_COLS: i32 = 80;
pub(crate) const PROF_NONE: i32 = 0;

pub struct EditorState {
    pub g_stats: NvimStats,
    pub rows_global: std::cell::Cell<i32>,
    pub columns_global: std::cell::Cell<i32>,
    pub mod_mask: std::cell::Cell<i32>,
    pub vgetc_mod_mask: std::cell::Cell<i32>,
    pub vgetc_char: std::cell::Cell<i32>,
    pub cmdline_row: std::cell::Cell<i32>,
    pub redraw_cmdline: std::cell::Cell<bool>,
    pub redraw_mode: std::cell::Cell<bool>,
    pub clear_cmdline: std::cell::Cell<bool>,
    pub mode_displayed: std::cell::Cell<bool>,
    pub cmdline_star: std::cell::Cell<i32>,
    pub redrawing_cmdline: std::cell::Cell<bool>,
    pub cmdline_was_last_drawn: std::cell::Cell<bool>,
    pub exec_from_reg: std::cell::Cell<bool>,
    pub dollar_vcol: std::cell::Cell<Colnr>,
    pub edit_submode: std::cell::Cell<Option<&'static std::ffi::CStr>>,
    pub edit_submode_pre: std::cell::Cell<Option<&'static std::ffi::CStr>>,
    pub edit_submode_extra: CloneCell<Option<std::ffi::CString>>,
    pub edit_submode_highl: std::cell::Cell<Hlf>,
    pub cmdmsg_rl: std::cell::Cell<bool>,
    pub msg_col: std::cell::Cell<i32>,
    pub msg_row: std::cell::Cell<i32>,
    pub msg_scrolled: std::cell::Cell<i32>,
    pub msg_scrolled_ign: std::cell::Cell<bool>,
    pub msg_did_scroll: std::cell::Cell<bool>,
    pub keep_msg: CloneCell<Option<std::ffi::CString>>,
    pub keep_msg_hl_id: std::cell::Cell<i32>,
    pub need_fileinfo: std::cell::Cell<bool>,
    pub msg_scroll: std::cell::Cell<i32>,
    pub msg_didout: std::cell::Cell<bool>,
    pub msg_didany: std::cell::Cell<bool>,
    pub msg_nowait: std::cell::Cell<bool>,
    pub emsg_off: std::cell::Cell<i32>,
    pub info_message: std::cell::Cell<bool>,
    pub msg_hist_off: std::cell::Cell<bool>,
    pub need_clr_eos: std::cell::Cell<bool>,
    pub emsg_skip: std::cell::Cell<i32>,
    pub emsg_severe: std::cell::Cell<bool>,
    pub emsg_assert_fails_msg: CloneCell<Option<std::ffi::CString>>,
    pub emsg_assert_fails_lnum: std::cell::Cell<i64>,
    pub emsg_assert_fails_context: CloneCell<Option<std::ffi::CString>>,
    pub did_endif: std::cell::Cell<bool>,
    pub did_emsg: std::cell::Cell<i32>,
    pub did_beep: std::cell::Cell<u64>,
    pub did_beep_silent: std::cell::Cell<u64>,
    pub did_emsg_syntax: std::cell::Cell<bool>,
    pub called_emsg: std::cell::Cell<i32>,
    pub ex_exitval: std::cell::Cell<i32>,
    pub emsg_on_display: std::cell::Cell<bool>,
    pub rc_did_emsg: std::cell::Cell<bool>,
    pub no_wait_return: std::cell::Cell<i32>,
    pub need_wait_return: std::cell::Cell<bool>,
    pub did_wait_return: std::cell::Cell<bool>,
    pub need_maketitle: std::cell::Cell<bool>,
    pub quit_more: std::cell::Cell<bool>,
    pub vgetc_busy: std::cell::Cell<i32>,
    pub didset_vim: std::cell::Cell<bool>,
    pub didset_vimruntime: std::cell::Cell<bool>,
    pub lines_left: std::cell::Cell<i32>,
    pub msg_no_more: std::cell::Cell<bool>,
    pub ex_nesting_level: std::cell::Cell<i32>,
    pub debug_break_level: std::cell::Cell<i32>,
    pub debug_did_msg: std::cell::Cell<bool>,
    pub debug_tick: std::cell::Cell<i32>,
    pub debug_backtrace_level: std::cell::Cell<i32>,
    pub do_profiling: std::cell::Cell<i32>,
    pub current_exception: CloneCell<Option<std::rc::Rc<Except>>>,
    pub did_throw: std::cell::Cell<bool>,
    pub need_rethrow: std::cell::Cell<bool>,
    pub check_cstack: std::cell::Cell<bool>,
    pub trylevel: std::cell::Cell<i32>,
    pub force_abort: std::cell::Cell<bool>,
    pub msg_list: std::cell::RefCell<Option<Vec<Msglist>>>,
    pub suppress_errthrow: std::cell::Cell<bool>,
    pub caught_stack: std::cell::RefCell<Vec<std::rc::Rc<Except>>>,
    pub may_garbage_collect: std::cell::Cell<bool>,
    pub want_garbage_collect: std::cell::Cell<bool>,
    pub garbage_collect_at_exit: std::cell::Cell<bool>,
    pub current_sctx: std::cell::Cell<Sctx>,
    pub current_ui: std::cell::Cell<u64>,
    pub did_source_packages: std::cell::Cell<bool>,
    pub provider_caller_scope: std::cell::RefCell<CallerScope>,
    pub provider_call_nesting: std::cell::Cell<i32>,
    pub t_colors: std::cell::Cell<i32>,
    pub include_none: std::cell::Cell<i32>,
    pub include_default: std::cell::Cell<i32>,
    pub include_link: std::cell::Cell<i32>,
    pub search_global: SearchState,
    pub need_check_timestamps: std::cell::Cell<bool>,
    pub did_check_timestamps: std::cell::Cell<bool>,
    pub no_check_timestamps: std::cell::Cell<i32>,
    pub mouse_grid: std::cell::Cell<i32>,
    pub mouse_row: std::cell::Cell<i32>,
    pub mouse_col: std::cell::Cell<i32>,
    pub mouse_past_bottom: std::cell::Cell<bool>,
    pub mouse_past_eol: std::cell::Cell<bool>,
    pub mouse_dragging: std::cell::Cell<i32>,
    pub root_menu: std::cell::RefCell<Vec<Vimmenu>>,
    pub sys_menu: std::cell::Cell<bool>,
    pub curwin_rc: CloneCell<WinPtr>,
    pub firstwin: CloneCell<WinPtr>,
    pub lastwin: CloneCell<WinPtr>,
    pub prevwin: CloneCell<WinPtr>,
    pub topframe: CloneCell<FramePtr>,
    pub first_tabpage: CloneCell<TabpagePtr>,
    pub curtab: CloneCell<TabpagePtr>,
    pub lastused_tabpage: CloneCell<TabpagePtr>,
    pub redraw_tabline: std::cell::Cell<bool>,
    pub firstbuf: CloneCell<BufPtr>,
    pub lastbuf: CloneCell<BufPtr>,
    pub curbuf: CloneCell<BufPtr>,
    pub arg_had_last: std::cell::Cell<bool>,
    pub ru_col: std::cell::Cell<i32>,
    pub ru_wid: std::cell::Cell<i32>,
    pub sc_col: std::cell::Cell<i32>,
    pub exiting: std::cell::Cell<bool>,
    pub v_dying: std::cell::Cell<i32>,
    pub stdin_isatty: std::cell::Cell<bool>,
    pub stdout_isatty: std::cell::Cell<bool>,
    pub stderr_isatty: std::cell::Cell<bool>,
    pub stdin_fd: std::cell::Cell<i32>,
    pub full_screen: std::cell::Cell<bool>,
    pub secure: std::cell::Cell<i32>,
    pub textlock: std::cell::Cell<i32>,
    pub allbuf_lock: std::cell::Cell<i32>,
    pub sandbox: std::cell::Cell<i32>,
    pub silent_mode: std::cell::Cell<bool>,
    pub exmode_active: std::cell::Cell<bool>,
    pub visual_global: std::cell::RefCell<VisualState>,
    pub where_paste_started: std::cell::Cell<CursorPos>,
    pub did_syncbind: std::cell::Cell<bool>,
    pub old_indent: std::cell::Cell<i32>,
    pub saved_cursor: std::cell::Cell<CursorPos>,
    pub ins_global: std::cell::RefCell<InsState>,
    pub orig_line_count: std::cell::Cell<Linenr>,
    pub vr_lines_changed: std::cell::Cell<i32>,
    pub inhibit_delete_count: std::cell::Cell<i32>,
    pub fenc_default: CloneCell<Option<std::ffi::CString>>,
    pub state_global: std::cell::Cell<i32>,
    pub debug_mode: std::cell::Cell<bool>,
    pub finish_op: std::cell::Cell<bool>,
    pub opcount: std::cell::Cell<i32>,
    pub motion_force: std::cell::Cell<i32>,
    pub cmdpreview: std::cell::Cell<bool>,
    pub reg_recording: std::cell::Cell<i32>,
    pub reg_executing: std::cell::Cell<i32>,
    pub pending_end_reg_executing: std::cell::Cell<bool>,
    pub reg_recorded: std::cell::Cell<i32>,
    pub no_mapping: std::cell::Cell<i32>,
    pub no_zero_mapping: std::cell::Cell<i32>,
    pub allow_keys: std::cell::Cell<i32>,
    pub no_u_sync: std::cell::Cell<i32>,
    pub u_sync_once: std::cell::Cell<i32>,
    pub force_restart_edit: std::cell::Cell<bool>,
    pub restart_edit: std::cell::Cell<i32>,
    pub ins_at_eol: std::cell::Cell<bool>,
    pub no_abbr: std::cell::Cell<bool>,
    pub mapped_ctrl_c: std::cell::Cell<i32>,
    pub ctrl_c_interrupts: std::cell::Cell<bool>,
    pub cmdmod: std::cell::RefCell<Cmdmod>,
    pub msg_silent: std::cell::Cell<i32>,
    pub emsg_silent: std::cell::Cell<i32>,
    pub emsg_noredir: std::cell::Cell<bool>,
    pub cmd_silent: std::cell::Cell<bool>,
    pub in_assert_fails: std::cell::Cell<bool>,
    pub swap_exists_action: std::cell::Cell<i32>,
    pub swap_exists_did_quit: std::cell::Cell<bool>,
    pub redrawing_disabled: std::cell::Cell<i32>,
    pub readonlymode: std::cell::Cell<bool>,
    pub recoverymode: std::cell::Cell<bool>,
    pub typebuf: Typebuf,
    pub typebuf_was_empty: std::cell::Cell<bool>,
    pub ex_normal_busy: std::cell::Cell<i32>,
    pub expr_map_lock: std::cell::Cell<i32>,
    pub ignore_script: std::cell::Cell<bool>,
    pub key_typed: std::cell::Cell<bool>,
    pub key_stuffed: std::cell::Cell<i32>,
    pub maptick: std::cell::Cell<i32>,
    pub must_redraw: std::cell::Cell<i32>,
    pub skip_redraw: std::cell::Cell<bool>,
    pub do_redraw: std::cell::Cell<bool>,
    pub must_redraw_pum: std::cell::Cell<bool>,
    pub need_highlight_changed: std::cell::Cell<bool>,
    pub scriptout: std::cell::RefCell<Option<crate::os::fs::CFile>>,
    pub got_int: std::cell::Cell<bool>,
    pub bangredo: std::cell::Cell<bool>,
    pub reg_do_extmatch: std::cell::Cell<i32>,
    pub re_extmatch_in: CloneCell<Option<std::rc::Rc<RegExtmatch>>>,
    pub re_extmatch_out: CloneCell<Option<std::rc::Rc<RegExtmatch>>>,
    pub did_outofmem_msg: std::cell::Cell<bool>,
    pub did_swapwrite_msg: std::cell::Cell<bool>,
    pub global_busy: std::cell::Cell<i32>,
    pub listcmd_busy: std::cell::Cell<bool>,
    pub need_start_insertmode: std::cell::Cell<bool>,
    pub last_mode: std::cell::RefCell<[u8; MODE_MAX_LENGTH]>,
    pub last_cmdline: CloneCell<Option<std::ffi::CString>>,
    pub repeat_cmdline: CloneCell<Option<std::ffi::CString>>,
    pub new_last_cmdline: CloneCell<Option<std::ffi::CString>>,
    pub postponed_split: std::cell::Cell<i32>,
    pub postponed_split_flags: std::cell::Cell<i32>,
    pub postponed_split_tab: std::cell::Cell<i32>,
    pub g_do_tagpreview: std::cell::Cell<i32>,
    pub g_tag_at_cursor: std::cell::Cell<bool>,
    pub replace_offset: std::cell::Cell<i32>,
    pub keep_help_flag: std::cell::Cell<bool>,
    pub redir_off: std::cell::Cell<bool>,
    pub redir_fd: std::cell::RefCell<Option<crate::os::fs::CFile>>,
    pub redir_reg: std::cell::Cell<i32>,
    pub redir_vname: std::cell::Cell<bool>,
    pub capture_ga: std::cell::RefCell<Option<Vec<u8>>>,
    pub langmap_mapchar: [std::cell::Cell<u8>; 256],
    pub save_p_ls: std::cell::Cell<i32>,
    pub save_p_wmh: std::cell::Cell<i32>,
    pub wild_menu_showing: std::cell::Cell<i32>,
    pub globaldir: CloneCell<Option<std::ffi::CString>>,
    pub last_chdir_reason: std::cell::Cell<Option<&'static std::ffi::CStr>>,
    pub km_stopsel: std::cell::Cell<bool>,
    pub km_startsel: std::cell::Cell<bool>,
    pub cmdwin_type: std::cell::Cell<i32>,
    pub cmdwin_buf: CloneCell<BufPtr>,
    pub cmdline_win: CloneCell<WinPtr>,
    pub sub_nsubs: std::cell::Cell<i32>,
    pub sub_nlines: std::cell::Cell<Linenr>,
    pub wim_flags: [std::cell::Cell<u8>; 4],
    pub stl_syntax: std::cell::Cell<i32>,
    pub typebuf_was_filled: std::cell::Cell<bool>,
    pub virtual_op: std::cell::Cell<TriState>,
    pub spell_redraw_lnum: std::cell::Cell<Linenr>,
    pub time_fd: std::cell::RefCell<Option<crate::os::fs::CFile>>,
    pub vim_ignored: std::cell::Cell<i32>,
    pub embedded_mode: std::cell::Cell<bool>,
    pub headless_mode: std::cell::Cell<bool>,
    pub windows_version: std::cell::RefCell<[u8; 20]>,
    pub skip_win_fix_scroll: std::cell::Cell<bool>,
    pub skip_update_topline: std::cell::Cell<bool>,
    pub namespace_ids: std::cell::RefCell<Vec<(std::ffi::CString, i32)>>,
    pub namespace_localscope: std::cell::RefCell<crate::memory::FxHashSet<u32>>,
    pub next_namespace_id: std::cell::Cell<ObjHandle>,
    pub buffer_handles: std::cell::RefCell<crate::memory::FxHashMap<i32, BufPtr>>,
    pub window_handles: std::cell::RefCell<crate::memory::FxHashMap<i32, WinPtr>>,
    pub tabpage_handles: std::cell::RefCell<crate::memory::FxHashMap<i32, TabpagePtr>>,
    pub last_cursormoved_win: CloneCell<WinPtr>,
    pub last_cursormoved: std::cell::Cell<CursorPos>,
    pub autocmd_busy: std::cell::Cell<bool>,
    pub autocmd_no_enter: std::cell::Cell<i32>,
    pub autocmd_no_leave: std::cell::Cell<i32>,
    pub au_new_curbuf: CloneCell<Bufref>,
    pub au_pending_free_buf: std::cell::RefCell<Vec<BufPtr>>,
    pub au_pending_free_win: std::cell::RefCell<Vec<WinPtr>>,
    pub autocmd_fname: CloneCell<Option<std::ffi::CString>>,
    pub autocmd_fname_full: std::cell::Cell<bool>,
    pub autocmd_bufnr: std::cell::Cell<i32>,
    pub autocmd_match: CloneCell<Option<std::ffi::CString>>,
    pub did_cursorhold: std::cell::Cell<bool>,
    pub deferred_events: CloneCell<MultiQueue>,
    pub on_print: Callback,
    pub ctx_win_vec: std::cell::RefCell<Vec<CtxWin>>,
    pub decor_state: std::cell::RefCell<DecorState>,
    pub diff_context: std::cell::Cell<i32>,
    pub diff_foldcolumn: std::cell::Cell<i32>,
    pub diff_need_scrollbind: std::cell::Cell<bool>,
    pub need_diff_redraw: std::cell::Cell<bool>,
    pub win_extmark_arr: std::cell::RefCell<Vec<WinExtmark>>,
    pub updating_screen: std::cell::Cell<bool>,
    pub redraw_not_allowed: std::cell::Cell<bool>,
    pub screen_search_hl: std::cell::RefCell<SynMatch>,
    pub search_hl_has_cursor_lnum: std::cell::Cell<Linenr>,
    pub curbuf_splice_pending: std::cell::Cell<i32>,
    pub disable_fold_update: std::cell::Cell<i32>,
    pub default_grid: ScreenGrid,
    pub default_gridview: GridView,
    pub resizing_screen: std::cell::Cell<bool>,
    pub linebuf_char: std::cell::RefCell<Vec<Schar>>,
    pub linebuf_attr: std::cell::RefCell<Vec<Sattr>>,
    pub linebuf_vcol: std::cell::RefCell<Vec<Colnr>>,
    pub highlight_attr: [std::cell::Cell<i32>; HLF_COUNT],
    pub highlight_attr_last: [std::cell::Cell<i32>; HLF_COUNT],
    pub highlight_user: [std::cell::Cell<i32>; 9],
    pub highlight_stlnc: [std::cell::Cell<i32>; 9],
    pub cterm_normal_fg_color: std::cell::Cell<i32>,
    pub cterm_normal_bg_color: std::cell::Cell<i32>,
    pub normal_fg: std::cell::Cell<RgbValue>,
    pub normal_bg: std::cell::Cell<RgbValue>,
    pub normal_sp: std::cell::Cell<RgbValue>,
    pub ns_hl_global: std::cell::Cell<NS>,
    pub ns_hl_win: std::cell::Cell<NS>,
    pub ns_hl_fast: std::cell::Cell<NS>,
    pub ns_hl_active: std::cell::Cell<NS>,
    pub hl_attr_active: CloneCell<Option<std::rc::Rc<crate::highlight::NSHlAttr>>>,
    pub nlua_disable_preload: std::cell::Cell<bool>,
    pub tslua_query_parse_count: std::cell::Cell<u64>,
    pub repeat_luamap: std::cell::Cell<i32>,
    pub namedfm: std::cell::RefCell<[Xfmark; 36]>,
    pub msg_ext_skip_flush: std::cell::Cell<bool>,
    pub msg_ext_overwrite: std::cell::Cell<bool>,
    pub msg_ext_skip_verbose: std::cell::Cell<bool>,
    pub msg_ext_fast: std::cell::Cell<bool>,
    pub msg_grid: ScreenGrid,
    pub msg_grid_pos: std::cell::Cell<i32>,
    pub msg_grid_adj: GridView,
    pub msg_scrolled_at_flush: std::cell::Cell<i32>,
    pub msg_grid_scroll_discount: std::cell::Cell<i32>,
    pub msg_listdo_overwrite: std::cell::Cell<i32>,
    pub ch_before_blocking_events: CloneCell<MultiQueue>,
    pub showcmd_buf: std::cell::RefCell<[u8; crate::normal::SHOWCMD_BUFLEN]>,
    pub nvim_testing: std::cell::Cell<bool>,
    pub pum_grid: ScreenGrid,
    pub pum_want: PumWant,
    pub last_insert_ga: std::cell::RefCell<Vec<u8>>,
    pub add_last_insert: std::cell::Cell<i32>,
    pub tab_page_click_defs: std::cell::RefCell<Vec<StlClickDefinition>>,
    pub ui_event_ns_id: std::cell::Cell<u32>,
    pub resize_events: CloneCell<MultiQueue>,
    pub ui_refresh_cmdheight: std::cell::Cell<bool>,
    pub grid_line_buf_char: std::cell::RefCell<Vec<Schar>>,
    pub grid_line_buf_attr: std::cell::RefCell<Vec<Sattr>>,
    pub ui_client_channel_id: std::cell::Cell<u64>,
    pub ui_client_error_exit: std::cell::Cell<i32>,
    pub ui_client_exit_status: std::cell::Cell<i32>,
    pub ui_client_attached: std::cell::Cell<bool>,
    pub ui_client_forward_stdin: std::cell::Cell<bool>,
    pub tabpage_move_disallowed: std::cell::Cell<i32>,
}

impl EditorState {
    pub const fn new() -> Self {
        Self {
            g_stats: NvimStats {
                fsync: std::cell::Cell::new(0),
                redraw: std::cell::Cell::new(0),
                log_skip: std::cell::Cell::new(0),
            },
            rows_global: std::cell::Cell::new(DFLT_ROWS),
            columns_global: std::cell::Cell::new(DFLT_COLS),
            mod_mask: std::cell::Cell::new(0),
            vgetc_mod_mask: std::cell::Cell::new(0),
            vgetc_char: std::cell::Cell::new(0),
            cmdline_row: std::cell::Cell::new(0),
            redraw_cmdline: std::cell::Cell::new(false),
            redraw_mode: std::cell::Cell::new(false),
            clear_cmdline: std::cell::Cell::new(false),
            mode_displayed: std::cell::Cell::new(false),
            cmdline_star: std::cell::Cell::new(0),
            redrawing_cmdline: std::cell::Cell::new(false),
            cmdline_was_last_drawn: std::cell::Cell::new(false),
            exec_from_reg: std::cell::Cell::new(false),
            dollar_vcol: std::cell::Cell::new(-1),
            edit_submode: std::cell::Cell::new(None),
            edit_submode_pre: std::cell::Cell::new(None),
            edit_submode_extra: CloneCell::new(None),
            edit_submode_highl: std::cell::Cell::new(0),
            cmdmsg_rl: std::cell::Cell::new(false),
            msg_col: std::cell::Cell::new(0),
            msg_row: std::cell::Cell::new(0),
            msg_scrolled: std::cell::Cell::new(0),
            msg_scrolled_ign: std::cell::Cell::new(false),
            msg_did_scroll: std::cell::Cell::new(false),
            keep_msg: CloneCell::new(None),
            keep_msg_hl_id: std::cell::Cell::new(0),
            need_fileinfo: std::cell::Cell::new(false),
            msg_scroll: std::cell::Cell::new(0),
            msg_didout: std::cell::Cell::new(false),
            msg_didany: std::cell::Cell::new(false),
            msg_nowait: std::cell::Cell::new(false),
            emsg_off: std::cell::Cell::new(0),
            info_message: std::cell::Cell::new(false),
            msg_hist_off: std::cell::Cell::new(false),
            need_clr_eos: std::cell::Cell::new(false),
            emsg_skip: std::cell::Cell::new(0),
            emsg_severe: std::cell::Cell::new(false),
            emsg_assert_fails_msg: CloneCell::new(None),
            emsg_assert_fails_lnum: std::cell::Cell::new(0),
            emsg_assert_fails_context: CloneCell::new(None),
            did_endif: std::cell::Cell::new(false),
            did_emsg: std::cell::Cell::new(0),
            did_beep: std::cell::Cell::new(0),
            did_beep_silent: std::cell::Cell::new(0),
            did_emsg_syntax: std::cell::Cell::new(false),
            called_emsg: std::cell::Cell::new(0),
            ex_exitval: std::cell::Cell::new(0),
            emsg_on_display: std::cell::Cell::new(false),
            rc_did_emsg: std::cell::Cell::new(false),
            no_wait_return: std::cell::Cell::new(0),
            need_wait_return: std::cell::Cell::new(false),
            did_wait_return: std::cell::Cell::new(false),
            need_maketitle: std::cell::Cell::new(true),
            quit_more: std::cell::Cell::new(false),
            vgetc_busy: std::cell::Cell::new(0),
            didset_vim: std::cell::Cell::new(false),
            didset_vimruntime: std::cell::Cell::new(false),
            lines_left: std::cell::Cell::new(-1),
            msg_no_more: std::cell::Cell::new(false),
            ex_nesting_level: std::cell::Cell::new(0),
            debug_break_level: std::cell::Cell::new(-1),
            debug_did_msg: std::cell::Cell::new(false),
            debug_tick: std::cell::Cell::new(0),
            debug_backtrace_level: std::cell::Cell::new(0),
            do_profiling: std::cell::Cell::new(PROF_NONE),
            current_exception: CloneCell::new(None),
            did_throw: std::cell::Cell::new(false),
            need_rethrow: std::cell::Cell::new(false),
            check_cstack: std::cell::Cell::new(false),
            trylevel: std::cell::Cell::new(0),
            force_abort: std::cell::Cell::new(false),
            msg_list: std::cell::RefCell::new(None),
            suppress_errthrow: std::cell::Cell::new(false),
            caught_stack: std::cell::RefCell::new(Vec::new()),
            may_garbage_collect: std::cell::Cell::new(false),
            want_garbage_collect: std::cell::Cell::new(false),
            garbage_collect_at_exit: std::cell::Cell::new(false),
            current_sctx: std::cell::Cell::new(Sctx {
                sc_sid: 0,
                sc_seq: 0,
                sc_lnum: 0,
                sc_chan: 0,
            }),
            current_ui: std::cell::Cell::new(0),
            did_source_packages: std::cell::Cell::new(false),
            provider_caller_scope: std::cell::RefCell::new(CallerScope {
                script_ctx: Sctx {
                    sc_sid: 0,
                    sc_seq: 0,
                    sc_lnum: 0,
                    sc_chan: 0,
                },
                es_entry: crate::profile::Estack {
                    es_lnum: 0,
                    es_name: None,
                    es_type: 0,
                    es_info: crate::profile::EStackInfo::None,
                },
                autocmd_fname: None,
                autocmd_match: None,
                autocmd_fname_full: false,
                autocmd_bufnr: 0,
                funccalp: crate::eval::userfunc::FunccalId::NULL,
            }),
            provider_call_nesting: std::cell::Cell::new(0),
            t_colors: std::cell::Cell::new(256),
            include_none: std::cell::Cell::new(0),
            include_default: std::cell::Cell::new(0),
            include_link: std::cell::Cell::new(0),
            search_global: SearchState {
                hl_match: std::cell::Cell::new(false),
                match_lines: std::cell::Cell::new(0),
                match_endcol: std::cell::Cell::new(0),
                first_line: std::cell::Cell::new(0),
                last_line: std::cell::Cell::new(MAXLNUM),
                no_smartcase: std::cell::Cell::new(false),
                cmdlen: std::cell::Cell::new(0),
                no_hlsearch: std::cell::Cell::new(false),
            },
            need_check_timestamps: std::cell::Cell::new(false),
            did_check_timestamps: std::cell::Cell::new(false),
            no_check_timestamps: std::cell::Cell::new(0),
            mouse_grid: std::cell::Cell::new(0),
            mouse_row: std::cell::Cell::new(0),
            mouse_col: std::cell::Cell::new(0),
            mouse_past_bottom: std::cell::Cell::new(false),
            mouse_past_eol: std::cell::Cell::new(false),
            mouse_dragging: std::cell::Cell::new(0),
            root_menu: std::cell::RefCell::new(Vec::new()),
            sys_menu: std::cell::Cell::new(false),
            curwin_rc: CloneCell::new(WinPtr::NULL),
            firstwin: CloneCell::new(WinPtr::NULL),
            lastwin: CloneCell::new(WinPtr::NULL),
            prevwin: CloneCell::new(WinPtr::NULL),
            topframe: CloneCell::new(FramePtr::NULL),
            first_tabpage: CloneCell::new(TabpagePtr::NULL),
            curtab: CloneCell::new(TabpagePtr::NULL),
            lastused_tabpage: CloneCell::new(TabpagePtr::NULL),
            redraw_tabline: std::cell::Cell::new(false),
            firstbuf: CloneCell::new(BufPtr::NULL),
            lastbuf: CloneCell::new(BufPtr::NULL),
            curbuf: CloneCell::new(BufPtr::NULL),
            arg_had_last: std::cell::Cell::new(false),
            ru_col: std::cell::Cell::new(0),
            ru_wid: std::cell::Cell::new(0),
            sc_col: std::cell::Cell::new(0),
            exiting: std::cell::Cell::new(false),
            v_dying: std::cell::Cell::new(0),
            stdin_isatty: std::cell::Cell::new(true),
            stdout_isatty: std::cell::Cell::new(true),
            stderr_isatty: std::cell::Cell::new(true),
            stdin_fd: std::cell::Cell::new(-1),
            full_screen: std::cell::Cell::new(false),
            secure: std::cell::Cell::new(0),
            textlock: std::cell::Cell::new(0),
            allbuf_lock: std::cell::Cell::new(0),
            sandbox: std::cell::Cell::new(0),
            silent_mode: std::cell::Cell::new(false),
            exmode_active: std::cell::Cell::new(false),
            visual_global: std::cell::RefCell::new(VisualState {
                start: CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                },
                active: false,
                select: false,
                select_reg: 0,
                select_exclu_adj: false,
                restart_select: 0,
                need_end: false,
                reselect: 0,
                mode: b'v' as i32,
                resel: crate::normal::VisualExtent {
                    mode: 0,
                    line_count: 0,
                    vcol: 0,
                },
            }),
            where_paste_started: std::cell::Cell::new(CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            }),
            did_syncbind: std::cell::Cell::new(false),
            old_indent: std::cell::Cell::new(0),
            saved_cursor: std::cell::Cell::new(CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            }),
            ins_global: std::cell::RefCell::new(InsState {
                start: CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                },
                start_orig: CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                },
                start_textlen: 0,
                start_blank_vcol: 0,
                moved: 0,
                stop_insert_mode: false,
                can_cindent: false,
                need_undo: false,
                did_ai: false,
                ai_col: 0,
                end_comment_pending: 0,
                did_si: false,
                can_si: false,
                can_si_back: false,
                update_start_orig: false,
                new_insert_skip: 0,
                did_restart_edit: 0,
                revins_on: false,
                revins_chars: 0,
                revins_legal: 0,
                revins_scol: 0,
                dont_sync_undo: crate::extmark::K_NONE,
                o_lnum: 0,
            }),
            orig_line_count: std::cell::Cell::new(0),
            vr_lines_changed: std::cell::Cell::new(0),
            inhibit_delete_count: std::cell::Cell::new(0),
            fenc_default: CloneCell::new(None),
            state_global: std::cell::Cell::new(MODE_NORMAL),
            debug_mode: std::cell::Cell::new(false),
            finish_op: std::cell::Cell::new(false),
            opcount: std::cell::Cell::new(0),
            motion_force: std::cell::Cell::new(0),
            cmdpreview: std::cell::Cell::new(false),
            reg_recording: std::cell::Cell::new(0),
            reg_executing: std::cell::Cell::new(0),
            pending_end_reg_executing: std::cell::Cell::new(false),
            reg_recorded: std::cell::Cell::new(0),
            no_mapping: std::cell::Cell::new(0),
            no_zero_mapping: std::cell::Cell::new(0),
            allow_keys: std::cell::Cell::new(0),
            no_u_sync: std::cell::Cell::new(0),
            u_sync_once: std::cell::Cell::new(0),
            force_restart_edit: std::cell::Cell::new(false),
            restart_edit: std::cell::Cell::new(0),
            ins_at_eol: std::cell::Cell::new(false),
            no_abbr: std::cell::Cell::new(true),
            mapped_ctrl_c: std::cell::Cell::new(0),
            ctrl_c_interrupts: std::cell::Cell::new(true),
            cmdmod: std::cell::RefCell::new(Cmdmod::new()),
            msg_silent: std::cell::Cell::new(0),
            emsg_silent: std::cell::Cell::new(0),
            emsg_noredir: std::cell::Cell::new(false),
            cmd_silent: std::cell::Cell::new(false),
            in_assert_fails: std::cell::Cell::new(false),
            swap_exists_action: std::cell::Cell::new(SEA_NONE),
            swap_exists_did_quit: std::cell::Cell::new(false),
            redrawing_disabled: std::cell::Cell::new(0),
            readonlymode: std::cell::Cell::new(false),
            recoverymode: std::cell::Cell::new(false),
            typebuf: Typebuf::ZERO,
            typebuf_was_empty: std::cell::Cell::new(false),
            ex_normal_busy: std::cell::Cell::new(0),
            expr_map_lock: std::cell::Cell::new(0),
            ignore_script: std::cell::Cell::new(false),
            key_typed: std::cell::Cell::new(false),
            key_stuffed: std::cell::Cell::new(0),
            maptick: std::cell::Cell::new(0),
            must_redraw: std::cell::Cell::new(0),
            skip_redraw: std::cell::Cell::new(false),
            do_redraw: std::cell::Cell::new(false),
            must_redraw_pum: std::cell::Cell::new(false),
            need_highlight_changed: std::cell::Cell::new(true),
            scriptout: std::cell::RefCell::new(None),
            got_int: std::cell::Cell::new(false),
            bangredo: std::cell::Cell::new(false),
            reg_do_extmatch: std::cell::Cell::new(0),
            re_extmatch_in: CloneCell::new(None),
            re_extmatch_out: CloneCell::new(None),
            did_outofmem_msg: std::cell::Cell::new(false),
            did_swapwrite_msg: std::cell::Cell::new(false),
            global_busy: std::cell::Cell::new(0),
            listcmd_busy: std::cell::Cell::new(false),
            need_start_insertmode: std::cell::Cell::new(false),
            last_mode: std::cell::RefCell::new([b'n', 0, 0, 0]),
            last_cmdline: CloneCell::new(None),
            repeat_cmdline: CloneCell::new(None),
            new_last_cmdline: CloneCell::new(None),
            postponed_split: std::cell::Cell::new(0),
            postponed_split_flags: std::cell::Cell::new(0),
            postponed_split_tab: std::cell::Cell::new(0),
            g_do_tagpreview: std::cell::Cell::new(0),
            g_tag_at_cursor: std::cell::Cell::new(false),
            replace_offset: std::cell::Cell::new(0),
            keep_help_flag: std::cell::Cell::new(false),
            redir_off: std::cell::Cell::new(false),
            redir_fd: std::cell::RefCell::new(None),
            redir_reg: std::cell::Cell::new(0),
            redir_vname: std::cell::Cell::new(false),
            capture_ga: std::cell::RefCell::new(None),
            langmap_mapchar: [const { std::cell::Cell::new(0) }; 256],
            save_p_ls: std::cell::Cell::new(-1),
            save_p_wmh: std::cell::Cell::new(-1),
            wild_menu_showing: std::cell::Cell::new(0),
            globaldir: CloneCell::new(None),
            last_chdir_reason: std::cell::Cell::new(None),
            km_stopsel: std::cell::Cell::new(false),
            km_startsel: std::cell::Cell::new(false),
            cmdwin_type: std::cell::Cell::new(0),
            cmdwin_buf: CloneCell::new(BufPtr::NULL),
            cmdline_win: CloneCell::new(WinPtr::NULL),
            sub_nsubs: std::cell::Cell::new(0),
            sub_nlines: std::cell::Cell::new(0),
            wim_flags: [const { std::cell::Cell::new(0) }; 4],
            stl_syntax: std::cell::Cell::new(0),
            typebuf_was_filled: std::cell::Cell::new(false),
            virtual_op: std::cell::Cell::new(-1),
            spell_redraw_lnum: std::cell::Cell::new(0),
            time_fd: std::cell::RefCell::new(None),
            vim_ignored: std::cell::Cell::new(0),
            embedded_mode: std::cell::Cell::new(false),
            headless_mode: std::cell::Cell::new(false),
            windows_version: std::cell::RefCell::new([0u8; 20]),
            skip_win_fix_scroll: std::cell::Cell::new(false),
            skip_update_topline: std::cell::Cell::new(false),
            namespace_ids: std::cell::RefCell::new(Vec::new()),
            namespace_localscope: std::cell::RefCell::new(crate::memory::fx_hashset()),
            next_namespace_id: std::cell::Cell::new(1),
            buffer_handles: std::cell::RefCell::new(crate::memory::fx_hashmap()),
            window_handles: std::cell::RefCell::new(crate::memory::fx_hashmap()),
            tabpage_handles: std::cell::RefCell::new(crate::memory::fx_hashmap()),
            last_cursormoved_win: CloneCell::new(WinPtr::NULL),
            last_cursormoved: std::cell::Cell::new(CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            }),
            autocmd_busy: std::cell::Cell::new(false),
            autocmd_no_enter: std::cell::Cell::new(0),
            autocmd_no_leave: std::cell::Cell::new(0),
            au_new_curbuf: CloneCell::new(Bufref {
                br_buf: crate::globals::BufPtr::NULL,
                br_fnum: 0,
                br_buf_free_count: 0,
            }),
            au_pending_free_buf: std::cell::RefCell::new(Vec::new()),
            au_pending_free_win: std::cell::RefCell::new(Vec::new()),
            autocmd_fname: CloneCell::new(None),
            autocmd_fname_full: std::cell::Cell::new(false),
            autocmd_bufnr: std::cell::Cell::new(0),
            autocmd_match: CloneCell::new(None),
            did_cursorhold: std::cell::Cell::new(true),
            deferred_events: CloneCell::new(MultiQueue::NULL),
            on_print: Callback::NONE,
            ctx_win_vec: std::cell::RefCell::new(Vec::new()),
            decor_state: std::cell::RefCell::new(DecorState::new()),
            diff_context: std::cell::Cell::new(6),
            diff_foldcolumn: std::cell::Cell::new(2),
            diff_need_scrollbind: std::cell::Cell::new(false),
            need_diff_redraw: std::cell::Cell::new(false),
            win_extmark_arr: std::cell::RefCell::new(Vec::new()),
            updating_screen: std::cell::Cell::new(false),
            redraw_not_allowed: std::cell::Cell::new(false),
            screen_search_hl: std::cell::RefCell::new(SynMatch {
                rm: crate::regexp::Regmmatch {
                    regprog: crate::regexp::RegprogPtr::NULL,
                    startpos: [crate::cursor::Lpos { lnum: 0, col: 0 }; crate::regexp::NSUBEXP],
                    endpos: [crate::cursor::Lpos { lnum: 0, col: 0 }; crate::regexp::NSUBEXP],
                    rmm_matchcol: 0,
                    rmm_ic: false,
                    rmm_maxcol: 0,
                },
                buf: BufPtr::NULL,
                lnum: 0,
                attr: 0,
                attr_cur: 0,
                first_lnum: 0,
                startcol: 0,
                endcol: 0,
                is_addpos: false,
                has_cursor: false,
                tm: 0,
            }),
            search_hl_has_cursor_lnum: std::cell::Cell::new(0),
            curbuf_splice_pending: std::cell::Cell::new(0),
            disable_fold_update: std::cell::Cell::new(0),
            default_grid: ScreenGrid::with_target(GridTarget::Default),
            default_gridview: GridView {
                target: std::cell::Cell::new(GridTarget::Default),
                row_offset: std::cell::Cell::new(0),
                col_offset: std::cell::Cell::new(0),
            },
            resizing_screen: std::cell::Cell::new(false),
            linebuf_char: std::cell::RefCell::new(Vec::new()),
            linebuf_attr: std::cell::RefCell::new(Vec::new()),
            linebuf_vcol: std::cell::RefCell::new(Vec::new()),
            highlight_attr: [const { std::cell::Cell::new(0) }; HLF_COUNT],
            highlight_attr_last: [const { std::cell::Cell::new(0) }; HLF_COUNT],
            highlight_user: [const { std::cell::Cell::new(0) }; 9],
            highlight_stlnc: [const { std::cell::Cell::new(0) }; 9],
            cterm_normal_fg_color: std::cell::Cell::new(0),
            cterm_normal_bg_color: std::cell::Cell::new(0),
            normal_fg: std::cell::Cell::new(-1),
            normal_bg: std::cell::Cell::new(-1),
            normal_sp: std::cell::Cell::new(-1),
            ns_hl_global: std::cell::Cell::new(0),
            ns_hl_win: std::cell::Cell::new(-1),
            ns_hl_fast: std::cell::Cell::new(-1),
            ns_hl_active: std::cell::Cell::new(0),
            hl_attr_active: CloneCell::new(None),
            nlua_disable_preload: std::cell::Cell::new(false),
            tslua_query_parse_count: std::cell::Cell::new(0),
            repeat_luamap: std::cell::Cell::new(0),
            namedfm: std::cell::RefCell::new([const { Xfmark::ZERO }; 36]),
            msg_ext_skip_flush: std::cell::Cell::new(false),
            msg_ext_overwrite: std::cell::Cell::new(false),
            msg_ext_skip_verbose: std::cell::Cell::new(false),
            msg_ext_fast: std::cell::Cell::new(true),
            msg_grid: ScreenGrid::with_target(GridTarget::Msg),
            msg_grid_pos: std::cell::Cell::new(0),
            msg_grid_adj: GridView {
                target: std::cell::Cell::new(GridTarget::None),
                row_offset: std::cell::Cell::new(0),
                col_offset: std::cell::Cell::new(0),
            },
            msg_scrolled_at_flush: std::cell::Cell::new(0),
            msg_grid_scroll_discount: std::cell::Cell::new(0),
            msg_listdo_overwrite: std::cell::Cell::new(0),
            ch_before_blocking_events: CloneCell::new(MultiQueue::NULL),
            showcmd_buf: std::cell::RefCell::new([0; crate::normal::SHOWCMD_BUFLEN]),
            nvim_testing: std::cell::Cell::new(false),
            pum_grid: ScreenGrid::with_target(GridTarget::Pum),
            pum_want: PumWant {
                active: std::cell::Cell::new(false),
                item: std::cell::Cell::new(0),
                insert: std::cell::Cell::new(false),
                finish: std::cell::Cell::new(false),
            },
            last_insert_ga: std::cell::RefCell::new(Vec::new()),
            add_last_insert: std::cell::Cell::new(0),
            tab_page_click_defs: std::cell::RefCell::new(Vec::new()),
            ui_event_ns_id: std::cell::Cell::new(0),
            resize_events: CloneCell::new(MultiQueue::NULL),
            ui_refresh_cmdheight: std::cell::Cell::new(true),
            grid_line_buf_char: std::cell::RefCell::new(Vec::new()),
            grid_line_buf_attr: std::cell::RefCell::new(Vec::new()),
            ui_client_channel_id: std::cell::Cell::new(0),
            ui_client_error_exit: std::cell::Cell::new(-1),
            ui_client_exit_status: std::cell::Cell::new(0),
            ui_client_attached: std::cell::Cell::new(false),
            ui_client_forward_stdin: std::cell::Cell::new(false),
            tabpage_move_disallowed: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static EDITOR_STATE: &'static EditorState = Box::leak(Box::new(EditorState::new()));
}

#[inline(always)]
pub fn g_st() -> &'static EditorState {
    EDITOR_STATE.with(|s| *s)
}

#[allow(non_upper_case_globals, unsafe_code)]
#[unsafe(export_name = "curwin")]
pub static CURWIN_FFI: std::sync::atomic::AtomicPtr<WinInfoInfo> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[inline(always)]
pub fn curwin() -> WinPtr {
    g_st().curwin_rc.get()
}

#[inline(always)]
pub fn set_curwin(wp: impl Into<WinPtr>) {
    let wp = wp.into();
    CURWIN_FFI.store(wp.as_ptr(), std::sync::atomic::Ordering::Relaxed);
    g_st().curwin_rc.set(wp);
}

#[inline(always)]
pub fn curbuf() -> BufPtr {
    g_st().curbuf.get()
}

#[inline(always)]
pub fn curbuf_opt() -> Option<BufPtr> {
    let b = g_st().curbuf.get();
    (!b.is_null()).then_some(b)
}

#[inline(always)]
pub fn curtab() -> TabpagePtr {
    g_st().curtab.get().clone()
}

#[inline(always)]
pub fn firstwin() -> WinPtr {
    g_st().firstwin.get()
}

#[inline(always)]
pub fn lastwin() -> WinPtr {
    g_st().lastwin.get()
}

#[inline(always)]
pub fn prevwin_opt() -> Option<WinPtr> {
    let w = g_st().prevwin.get();
    (!w.is_null()).then_some(w)
}

macro_rules! impl_rc_graph_ptr {
    ($ptr_name:ident, $target:ty) => {
        #[repr(transparent)]
        #[derive(Clone, Default)]
        pub struct $ptr_name(Option<std::rc::Rc<$target>>);

        impl PartialEq for $ptr_name {
            #[inline(always)]
            fn eq(&self, other: &Self) -> bool {
                match (&self.0, &other.0) {
                    (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
            }
        }

        impl Eq for $ptr_name {}

        impl PartialOrd for $ptr_name {
            #[inline(always)]
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for $ptr_name {
            #[inline(always)]
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.addr().cmp(&other.addr())
            }
        }

        impl PartialEq<&$ptr_name> for $ptr_name {
            #[inline(always)]
            fn eq(&self, other: &&$ptr_name) -> bool {
                self == *other
            }
        }

        impl PartialEq<$ptr_name> for &$ptr_name {
            #[inline(always)]
            fn eq(&self, other: &$ptr_name) -> bool {
                *self == other
            }
        }

        impl PartialEq<&$target> for $ptr_name {
            #[inline(always)]
            fn eq(&self, other: &&$target) -> bool {
                self.0
                    .as_ref()
                    .is_some_and(|rc| std::ptr::eq(rc.as_ref(), *other))
            }
        }

        impl PartialEq<&mut $target> for $ptr_name {
            #[inline(always)]
            fn eq(&self, other: &&mut $target) -> bool {
                self.0
                    .as_ref()
                    .is_some_and(|rc| std::ptr::eq(rc.as_ref(), *other))
            }
        }

        impl PartialEq<*mut $target> for $ptr_name {
            #[inline(always)]
            fn eq(&self, other: &*mut $target) -> bool {
                self.as_ptr() == *other
            }
        }

        impl PartialEq<*const $target> for $ptr_name {
            #[inline(always)]
            fn eq(&self, other: &*const $target) -> bool {
                self.as_ptr().cast_const() == *other
            }
        }

        impl PartialEq<$ptr_name> for &$target {
            #[inline(always)]
            fn eq(&self, other: &$ptr_name) -> bool {
                other == self
            }
        }

        impl PartialEq<$ptr_name> for &mut $target {
            #[inline(always)]
            fn eq(&self, other: &$ptr_name) -> bool {
                other == self
            }
        }

        impl PartialEq<$ptr_name> for *mut $target {
            #[inline(always)]
            fn eq(&self, other: &$ptr_name) -> bool {
                *self == other.as_ptr()
            }
        }

        impl PartialEq<$ptr_name> for *const $target {
            #[inline(always)]
            fn eq(&self, other: &$ptr_name) -> bool {
                *self == other.as_ptr().cast_const()
            }
        }

        impl std::fmt::Debug for $ptr_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_tuple(stringify!($ptr_name))
                    .field(&self.0.as_ref().map(std::rc::Rc::as_ptr))
                    .finish()
            }
        }

        impl $ptr_name {
            pub const NULL: Self = Self(None);

            #[inline(always)]
            pub const fn is_null(&self) -> bool {
                self.0.is_none()
            }

            #[inline(always)]
            pub fn opt(&self) -> Option<&$target> {
                self.0.as_deref()
            }

            #[inline(always)]
            pub fn as_ref(&self) -> Option<&$target> {
                self.0.as_deref()
            }

            #[inline(always)]
            pub fn borrow(&self) -> &$target {
                self.opt().expect(concat!("null ", stringify!($ptr_name)))
            }

            #[inline(always)]
            pub fn rc(&self) -> Option<std::rc::Rc<$target>> {
                self.0.clone()
            }

            #[inline(always)]
            pub fn unwrap_rc(&self) -> std::rc::Rc<$target> {
                self.0
                    .clone()
                    .expect(concat!("null ", stringify!($ptr_name)))
            }

            #[inline(always)]
            pub fn as_ptr(&self) -> *mut $target {
                self.0.as_ref().map_or(std::ptr::null_mut(), |rc| {
                    std::rc::Rc::as_ptr(rc).cast_mut()
                })
            }

            #[inline(always)]
            pub fn addr(&self) -> usize {
                self.as_ptr() as usize
            }
        }

        impl From<std::rc::Rc<$target>> for $ptr_name {
            #[inline(always)]
            fn from(rc: std::rc::Rc<$target>) -> Self {
                Self(Some(rc))
            }
        }

        impl From<&std::rc::Rc<$target>> for $ptr_name {
            #[inline(always)]
            fn from(rc: &std::rc::Rc<$target>) -> Self {
                Self(Some(rc.clone()))
            }
        }

        impl From<Option<std::rc::Rc<$target>>> for $ptr_name {
            #[inline(always)]
            fn from(opt: Option<std::rc::Rc<$target>>) -> Self {
                Self(opt)
            }
        }

        impl From<&$ptr_name> for $ptr_name {
            #[inline(always)]
            fn from(p: &$ptr_name) -> Self {
                p.clone()
            }
        }

        impl From<&&$ptr_name> for $ptr_name {
            #[inline(always)]
            fn from(p: &&$ptr_name) -> Self {
                (*p).clone()
            }
        }

        impl From<&&$target> for $ptr_name {
            #[inline(always)]
            fn from(r: &&$target) -> Self {
                Self(r.self_weak.upgrade())
            }
        }

        impl From<Option<$ptr_name>> for $ptr_name {
            #[inline(always)]
            fn from(opt: Option<$ptr_name>) -> Self {
                opt.unwrap_or(Self::NULL)
            }
        }

        impl From<&Option<$ptr_name>> for $ptr_name {
            #[inline(always)]
            fn from(opt: &Option<$ptr_name>) -> Self {
                opt.clone().unwrap_or(Self::NULL)
            }
        }

        impl From<&$target> for $ptr_name {
            #[inline(always)]
            fn from(r: &$target) -> Self {
                Self(r.self_weak.upgrade())
            }
        }

        impl From<&mut $target> for $ptr_name {
            #[inline(always)]
            fn from(r: &mut $target) -> Self {
                Self(r.self_weak.upgrade())
            }
        }

        impl From<Option<&$target>> for $ptr_name {
            #[inline(always)]
            fn from(opt: Option<&$target>) -> Self {
                Self(opt.and_then(|r| r.self_weak.upgrade()))
            }
        }

        impl From<Option<&mut $target>> for $ptr_name {
            #[inline(always)]
            fn from(opt: Option<&mut $target>) -> Self {
                Self(opt.and_then(|r| r.self_weak.upgrade()))
            }
        }

        impl std::ops::Deref for $ptr_name {
            type Target = $target;

            #[inline(always)]
            fn deref(&self) -> &Self::Target {
                self.borrow()
            }
        }

        impl AsRef<$target> for $ptr_name {
            #[inline(always)]
            fn as_ref(&self) -> &$target {
                self.borrow()
            }
        }

        impl std::borrow::Borrow<$target> for $ptr_name {
            #[inline(always)]
            fn borrow(&self) -> &$target {
                self.borrow()
            }
        }
    };
}

impl_rc_graph_ptr!(WinPtr, WinInfoInfo);
impl_rc_graph_ptr!(BufPtr, FileBuf);

#[repr(transparent)]
#[derive(Clone, Default)]
pub struct TabpagePtr(Option<std::rc::Rc<TabPageInfo>>);

impl PartialEq for TabpagePtr {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Eq for TabpagePtr {}

impl PartialEq<&TabPageInfo> for TabpagePtr {
    #[inline(always)]
    fn eq(&self, other: &&TabPageInfo) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| std::ptr::eq(rc.as_ref(), *other))
    }
}

impl PartialEq<&mut TabPageInfo> for TabpagePtr {
    #[inline(always)]
    fn eq(&self, other: &&mut TabPageInfo) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| std::ptr::eq(rc.as_ref(), *other))
    }
}

impl PartialEq<TabpagePtr> for &TabPageInfo {
    #[inline(always)]
    fn eq(&self, other: &TabpagePtr) -> bool {
        other == self
    }
}

impl PartialEq<TabpagePtr> for &mut TabPageInfo {
    #[inline(always)]
    fn eq(&self, other: &TabpagePtr) -> bool {
        other == self
    }
}

impl std::fmt::Debug for TabpagePtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TabpagePtr")
            .field(&self.0.as_ref().map(std::rc::Rc::as_ptr))
            .finish()
    }
}

impl TabpagePtr {
    pub const NULL: Self = Self(None);

    #[inline(always)]
    pub const fn is_null(&self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn opt(&self) -> Option<&TabPageInfo> {
        self.0.as_deref()
    }

    #[inline(always)]
    pub fn borrow(&self) -> &TabPageInfo {
        self.opt().expect("null TabpagePtr")
    }

    #[inline(always)]
    pub fn rc(&self) -> Option<std::rc::Rc<TabPageInfo>> {
        self.0.clone()
    }

    #[inline(always)]
    pub fn unwrap_rc(&self) -> std::rc::Rc<TabPageInfo> {
        self.0.clone().expect("null TabpagePtr")
    }
}

impl From<std::rc::Rc<TabPageInfo>> for TabpagePtr {
    #[inline(always)]
    fn from(rc: std::rc::Rc<TabPageInfo>) -> Self {
        Self(Some(rc))
    }
}

impl From<&std::rc::Rc<TabPageInfo>> for TabpagePtr {
    #[inline(always)]
    fn from(rc: &std::rc::Rc<TabPageInfo>) -> Self {
        Self(Some(rc.clone()))
    }
}

impl From<Option<std::rc::Rc<TabPageInfo>>> for TabpagePtr {
    #[inline(always)]
    fn from(opt: Option<std::rc::Rc<TabPageInfo>>) -> Self {
        Self(opt)
    }
}

impl From<&TabpagePtr> for TabpagePtr {
    #[inline(always)]
    fn from(p: &TabpagePtr) -> Self {
        p.clone()
    }
}

impl From<Option<TabpagePtr>> for TabpagePtr {
    #[inline(always)]
    fn from(opt: Option<TabpagePtr>) -> Self {
        opt.unwrap_or(Self::NULL)
    }
}

impl From<&TabPageInfo> for TabpagePtr {
    #[inline(always)]
    fn from(r: &TabPageInfo) -> Self {
        Self(r.self_weak.upgrade())
    }
}

impl From<&mut TabPageInfo> for TabpagePtr {
    #[inline(always)]
    fn from(r: &mut TabPageInfo) -> Self {
        Self(r.self_weak.upgrade())
    }
}

impl From<Option<&TabPageInfo>> for TabpagePtr {
    #[inline(always)]
    fn from(opt: Option<&TabPageInfo>) -> Self {
        Self(opt.and_then(|r| r.self_weak.upgrade()))
    }
}

impl From<Option<&mut TabPageInfo>> for TabpagePtr {
    #[inline(always)]
    fn from(opt: Option<&mut TabPageInfo>) -> Self {
        Self(opt.and_then(|r| r.self_weak.upgrade()))
    }
}

impl std::ops::Deref for TabpagePtr {
    type Target = TabPageInfo;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

#[repr(transparent)]
#[derive(Clone, Default)]
pub struct FramePtr(Option<std::rc::Rc<Frame>>);

impl PartialEq for FramePtr {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Eq for FramePtr {}

impl PartialEq<&Frame> for FramePtr {
    #[inline(always)]
    fn eq(&self, other: &&Frame) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| std::ptr::eq(rc.as_ref(), *other))
    }
}

impl PartialEq<&mut Frame> for FramePtr {
    #[inline(always)]
    fn eq(&self, other: &&mut Frame) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| std::ptr::eq(rc.as_ref(), *other))
    }
}

impl PartialEq<FramePtr> for &Frame {
    #[inline(always)]
    fn eq(&self, other: &FramePtr) -> bool {
        other == self
    }
}

impl PartialEq<FramePtr> for &mut Frame {
    #[inline(always)]
    fn eq(&self, other: &FramePtr) -> bool {
        other == self
    }
}

impl std::fmt::Debug for FramePtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FramePtr")
            .field(&self.0.as_ref().map(std::rc::Rc::as_ptr))
            .finish()
    }
}

impl FramePtr {
    pub const NULL: Self = Self(None);

    #[inline(always)]
    pub const fn is_null(&self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn opt(&self) -> Option<&Frame> {
        self.0.as_deref()
    }

    #[inline(always)]
    pub fn borrow(&self) -> &Frame {
        self.opt().expect("null FramePtr")
    }

    #[inline(always)]
    pub fn rc(&self) -> Option<std::rc::Rc<Frame>> {
        self.0.clone()
    }

    #[inline(always)]
    pub fn unwrap_rc(&self) -> std::rc::Rc<Frame> {
        self.0.clone().expect("null FramePtr")
    }
}

impl From<std::rc::Rc<Frame>> for FramePtr {
    #[inline(always)]
    fn from(rc: std::rc::Rc<Frame>) -> Self {
        Self(Some(rc))
    }
}

impl From<&std::rc::Rc<Frame>> for FramePtr {
    #[inline(always)]
    fn from(rc: &std::rc::Rc<Frame>) -> Self {
        Self(Some(rc.clone()))
    }
}

impl From<Option<std::rc::Rc<Frame>>> for FramePtr {
    #[inline(always)]
    fn from(opt: Option<std::rc::Rc<Frame>>) -> Self {
        Self(opt)
    }
}

impl From<&FramePtr> for FramePtr {
    #[inline(always)]
    fn from(p: &FramePtr) -> Self {
        p.clone()
    }
}

impl From<Option<FramePtr>> for FramePtr {
    #[inline(always)]
    fn from(opt: Option<FramePtr>) -> Self {
        opt.unwrap_or(Self::NULL)
    }
}

impl From<&Frame> for FramePtr {
    #[inline(always)]
    fn from(r: &Frame) -> Self {
        Self(r.self_weak.upgrade())
    }
}

impl From<&mut Frame> for FramePtr {
    #[inline(always)]
    fn from(r: &mut Frame) -> Self {
        Self(r.self_weak.upgrade())
    }
}

impl From<Option<&Frame>> for FramePtr {
    #[inline(always)]
    fn from(opt: Option<&Frame>) -> Self {
        Self(opt.and_then(|r| r.self_weak.upgrade()))
    }
}

impl From<Option<&mut Frame>> for FramePtr {
    #[inline(always)]
    fn from(opt: Option<&mut Frame>) -> Self {
        Self(opt.and_then(|r| r.self_weak.upgrade()))
    }
}

impl std::ops::Deref for FramePtr {
    type Target = Frame;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

pub struct BufPtrIter(BufPtr);

impl Iterator for BufPtrIter {
    type Item = BufPtr;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_null() {
            return None;
        }
        let next = self.0.b_next.get();
        Some(std::mem::replace(&mut self.0, next))
    }
}

#[inline(always)]
pub fn buf_iter_ptr(start: impl Into<BufPtr>) -> BufPtrIter {
    BufPtrIter(start.into())
}

#[inline(always)]
pub fn for_all_buffers_ptr() -> BufPtrIter {
    BufPtrIter(g_st().firstbuf.get())
}

pub struct BufRevPtrIter(BufPtr);

impl Iterator for BufRevPtrIter {
    type Item = BufPtr;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_null() {
            return None;
        }
        let prev = self.0.b_prev.get();
        Some(std::mem::replace(&mut self.0, prev))
    }
}

#[inline(always)]
pub fn for_all_buffers_rev_ptr() -> BufRevPtrIter {
    BufRevPtrIter(g_st().lastbuf.get())
}

pub struct WinPtrIter(WinPtr);

impl Iterator for WinPtrIter {
    type Item = WinPtr;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_null() {
            return None;
        }
        let next = self.0.w_next.get();
        Some(std::mem::replace(&mut self.0, next))
    }
}

#[inline(always)]
pub fn for_all_windows_ptr() -> WinPtrIter {
    WinPtrIter(g_st().firstwin.get())
}

#[inline(always)]
pub fn win_iter_ptr(start: impl Into<WinPtr>) -> WinPtrIter {
    WinPtrIter(start.into())
}

#[inline(always)]
pub fn for_all_windows_in_tab_ptr(tp: &TabPageInfo) -> WinPtrIter {
    WinPtrIter(if g_st().curtab.get() == tp {
        g_st().firstwin.get()
    } else {
        tp.tp_firstwin.get()
    })
}

pub struct WinRevPtrIter(WinPtr);

impl Iterator for WinRevPtrIter {
    type Item = WinPtr;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_null() {
            return None;
        }
        let prev = self.0.w_prev.get();
        Some(std::mem::replace(&mut self.0, prev))
    }
}

#[inline(always)]
pub fn for_all_windows_rev_ptr() -> WinRevPtrIter {
    WinRevPtrIter(g_st().lastwin.get())
}

pub struct TabPtrIter(TabpagePtr);

impl Iterator for TabPtrIter {
    type Item = TabpagePtr;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_null() {
            return None;
        }
        let next = self.0.tp_next.get();
        Some(std::mem::replace(&mut self.0, next))
    }
}

#[inline(always)]
pub fn for_all_tabpages_ptr() -> TabPtrIter {
    TabPtrIter(g_st().first_tabpage.get().clone())
}

pub struct FramePtrIter(FramePtr);

impl Iterator for FramePtrIter {
    type Item = FramePtr;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_null() {
            return None;
        }
        let next = self.0.fr_next.get();
        Some(std::mem::replace(&mut self.0, next))
    }
}

#[inline(always)]
pub fn frame_iter_ptr(start: impl Into<FramePtr>) -> FramePtrIter {
    FramePtrIter(start.into())
}

#[allow(non_upper_case_globals, unsafe_code)]
#[unsafe(no_mangle)]
pub static starting: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(NO_SCREEN);

#[inline(always)]
pub fn starting_val() -> i32 {
    starting.load(std::sync::atomic::Ordering::Relaxed)
}

#[inline(always)]
pub fn set_starting(v: i32) {
    starting.store(v, std::sync::atomic::Ordering::Relaxed);
}

pub const ESCAPE_CHARS: &std::ffi::CStr = c" \t\\\"|";
pub const NO_LINES_MSG: &std::ffi::CStr = c"--No lines in buffer--";
#[allow(non_upper_case_globals, unsafe_code)]
#[unsafe(no_mangle)]
pub static display_tick: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[inline(always)]
pub fn display_tick_val() -> Disptick {
    display_tick.load(std::sync::atomic::Ordering::Relaxed)
}

#[inline(always)]
pub fn inc_display_tick() {
    display_tick.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}
#[inline(always)]
pub fn has_time_fd() -> bool {
    g_st().time_fd.borrow().is_some()
}
#[inline(always)]
pub fn has_redir_fd() -> bool {
    g_st().redir_fd.borrow().is_some()
}
#[inline(always)]
pub fn has_scriptout() -> bool {
    g_st().scriptout.borrow().is_some()
}
#[allow(clippy::declare_interior_mutable_const)]
pub const SCREEN_GRID_INIT: ScreenGrid = ScreenGrid::with_target(GridTarget::None);
pub const UI_EXT_NAMES: [&std::ffi::CStr; 10] = [
    c"ext_cmdline",
    c"ext_popupmenu",
    c"ext_tabline",
    c"ext_wildmenu",
    c"ext_messages",
    c"ext_linegrid",
    c"ext_multigrid",
    c"ext_hlstate",
    c"ext_termcolors",
    c"_debug_float",
];
pub const MSG_LOCLIST: &std::ffi::CStr = c"[Location List]";
pub const MSG_QFLIST: &std::ffi::CStr = c"[Quickfix List]";
pub const VIRT_TEXT_POS_STR: [&std::ffi::CStr; 6] = [
    c"eol",
    c"eol_right_align",
    c"inline",
    c"overlay",
    c"right_align",
    c"win_col",
];
pub const HL_MODE_STR: [&std::ffi::CStr; 4] = [c"", c"replace", c"combine", c"blend"];
pub const HLF_NAMES: [Option<&std::ffi::CStr>; HLF_COUNT] = [
    None,
    Some(c"SpecialKey"),
    Some(c"EndOfBuffer"),
    Some(c"TermCursor"),
    Some(c"NonText"),
    Some(c"Directory"),
    Some(c"ErrorMsg"),
    Some(c"IncSearch"),
    Some(c"Search"),
    Some(c"CurSearch"),
    Some(c"MoreMsg"),
    Some(c"ModeMsg"),
    Some(c"LineNr"),
    Some(c"LineNrAbove"),
    Some(c"LineNrBelow"),
    Some(c"CursorLineNr"),
    Some(c"CursorLineSign"),
    Some(c"CursorLineFold"),
    Some(c"Question"),
    Some(c"StatusLine"),
    Some(c"StatusLineNC"),
    Some(c"WinSeparator"),
    Some(c"VertSplit"),
    Some(c"Title"),
    Some(c"Visual"),
    Some(c"VisualNC"),
    Some(c"WarningMsg"),
    Some(c"WildMenu"),
    Some(c"Folded"),
    Some(c"FoldColumn"),
    Some(c"DiffAdd"),
    Some(c"DiffChange"),
    Some(c"DiffDelete"),
    Some(c"DiffText"),
    Some(c"DiffTextAdd"),
    Some(c"SignColumn"),
    Some(c"Conceal"),
    Some(c"SpellBad"),
    Some(c"SpellCap"),
    Some(c"SpellRare"),
    Some(c"SpellLocal"),
    Some(c"Pmenu"),
    Some(c"PmenuSel"),
    Some(c"PmenuMatch"),
    Some(c"PmenuMatchSel"),
    Some(c"PmenuKind"),
    Some(c"PmenuKindSel"),
    Some(c"PmenuExtra"),
    Some(c"PmenuExtraSel"),
    Some(c"PmenuSbar"),
    Some(c"PmenuThumb"),
    Some(c"PmenuBorder"),
    Some(c"TabLine"),
    Some(c"TabLineSel"),
    Some(c"TabLineFill"),
    Some(c"CursorColumn"),
    Some(c"CursorLine"),
    Some(c"ColorColumn"),
    Some(c"QuickFixLine"),
    Some(c"Whitespace"),
    Some(c"NormalNC"),
    Some(c"MsgSeparator"),
    Some(c"NormalFloat"),
    Some(c"MsgArea"),
    Some(c"FloatBorder"),
    Some(c"WinBar"),
    Some(c"WinBarNC"),
    Some(c"Cursor"),
    Some(c"FloatTitle"),
    Some(c"FloatFooter"),
    Some(c"StatusLineTerm"),
    Some(c"StatusLineTermNC"),
    Some(c"StderrMsg"),
    Some(c"StdoutMsg"),
    Some(c"OkMsg"),
    Some(c"PreInsert"),
];
#[inline(always)]
pub const fn hlf_name_ptr(idx: usize) -> Option<&'static std::ffi::CStr> {
    HLF_NAMES[idx]
}
#[allow(non_upper_case_globals, unsafe_code)]
#[unsafe(no_mangle)]
pub static test_disable_char_avail: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
pub const FLOAT_ANCHOR_STR: [&std::ffi::CStr; 4] = [c"NW", c"NE", c"SW", c"SE"];

#[cfg(test)]
pub(crate) static TEST_GLOBAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[inline(always)]
pub fn topframe() -> FramePtr {
    g_st().topframe.get().clone()
}

#[inline(always)]
pub fn first_tabpage() -> TabpagePtr {
    g_st().first_tabpage.get().clone()
}

#[inline(always)]
pub fn nlua_global_refs() -> NluaRefState {
    crate::lua::executor::nlua_global_refs()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_globals_match_c_initializers() {
        let _guard = TEST_GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        {
            assert_eq!({ g_st().rows_global.get() }, 24);
            assert_eq!({ g_st().columns_global.get() }, 80);
            assert_eq!({ g_st().dollar_vcol.get() }, -1);
            assert_eq!({ g_st().t_colors.get() }, 256);
            assert_eq!(starting_val(), 2);
            assert_eq!({ g_st().state_global.get() }, 0x01);
            assert_eq!({ g_st().visual_global.borrow().mode }, b'v' as i32);
            assert_eq!({ g_st().search_global.last_line.get() }, 0x7fffffff);
            assert_eq!(NO_LINES_MSG.to_bytes(), b"--No lines in buffer--");
        }
    }
}
