//! Global variables (`src/nvim/globals.h`).

use crate::highlight::{HLF_COUNT, RgbValue};

use std::ptr;

use crate::decoration::{DecorState, NS};
use crate::ex_docmd::Cmdmod;

use crate::api::ui::NO_SCREEN;
use crate::buffer::{Bufref, FileBuf, MAXLNUM, SEA_NONE};
use crate::channel::Channel;
use crate::context::CtxWin;
use crate::cursor::{Colnr, CursorPos, Linenr};
use crate::drawscreen::WinExtmark;
use crate::eval::CallerScope;
use crate::eval::typval::Callback;
use crate::eval::userfunc::MSG_BUF_LEN;
use crate::eval::vars::Evalarg;
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
use crate::os::fs::{MAXPATHL, NvimStats};
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

use crate::event::r#loop::Loop;

#[repr(transparent)]
struct GlobalState<T>(std::cell::UnsafeCell<T>);

unsafe impl<T> Sync for GlobalState<T> {}

impl<T> GlobalState<T> {
    #[inline(always)]
    const fn new(val: T) -> Self {
        Self(std::cell::UnsafeCell::new(val))
    }

    #[inline(always)]
    const fn handle(&'static self) -> GraphPtr<T> {
        GraphPtr((&raw const self.0).cast::<T>().cast_mut())
    }
}

pub struct EditorState {
    pub main_loop: Loop,
    pub g_stats: NvimStats,
    pub rows_global: i32,
    pub columns_global: i32,
    pub mod_mask: i32,
    pub vgetc_mod_mask: i32,
    pub vgetc_char: i32,
    pub cmdline_row: i32,
    pub redraw_cmdline: bool,
    pub redraw_mode: bool,
    pub clear_cmdline: bool,
    pub mode_displayed: bool,
    pub cmdline_star: i32,
    pub redrawing_cmdline: bool,
    pub cmdline_was_last_drawn: bool,
    pub exec_from_reg: bool,
    pub dollar_vcol: Colnr,
    pub edit_submode: Option<&'static std::ffi::CStr>,
    pub edit_submode_pre: Option<&'static std::ffi::CStr>,
    pub edit_submode_extra: Option<std::ffi::CString>,
    pub edit_submode_highl: Hlf,
    pub cmdmsg_rl: bool,
    pub msg_col: i32,
    pub msg_row: i32,
    pub msg_scrolled: i32,
    pub msg_scrolled_ign: bool,
    pub msg_did_scroll: bool,
    pub keep_msg: Option<std::ffi::CString>,
    pub keep_msg_hl_id: i32,
    pub need_fileinfo: bool,
    pub msg_scroll: i32,
    pub msg_didout: bool,
    pub msg_didany: bool,
    pub msg_nowait: bool,
    pub emsg_off: i32,
    pub info_message: bool,
    pub msg_hist_off: bool,
    pub need_clr_eos: bool,
    pub emsg_skip: i32,
    pub emsg_severe: bool,
    pub emsg_assert_fails_msg: Option<std::ffi::CString>,
    pub emsg_assert_fails_lnum: i64,
    pub emsg_assert_fails_context: Option<std::ffi::CString>,
    pub did_endif: bool,
    pub did_emsg: i32,
    pub did_beep: u64,
    pub did_beep_silent: u64,
    pub did_emsg_syntax: bool,
    pub called_emsg: i32,
    pub ex_exitval: i32,
    pub emsg_on_display: bool,
    pub rc_did_emsg: bool,
    pub no_wait_return: i32,
    pub need_wait_return: bool,
    pub did_wait_return: bool,
    pub need_maketitle: bool,
    pub quit_more: bool,
    pub vgetc_busy: i32,
    pub didset_vim: bool,
    pub didset_vimruntime: bool,
    pub lines_left: i32,
    pub msg_no_more: bool,
    pub ex_nesting_level: i32,
    pub debug_break_level: i32,
    pub debug_did_msg: bool,
    pub debug_tick: i32,
    pub debug_backtrace_level: i32,
    pub do_profiling: i32,
    pub current_exception: Option<std::rc::Rc<Except>>,
    pub did_throw: bool,
    pub need_rethrow: bool,
    pub check_cstack: bool,
    pub trylevel: i32,
    pub force_abort: bool,
    pub msg_list: Option<Vec<Msglist>>,
    pub suppress_errthrow: bool,
    pub caught_stack: Vec<std::rc::Rc<Except>>,
    pub may_garbage_collect: bool,
    pub want_garbage_collect: bool,
    pub garbage_collect_at_exit: bool,
    pub current_sctx: Sctx,
    pub current_ui: u64,
    pub did_source_packages: bool,
    pub provider_caller_scope: CallerScope,
    pub provider_call_nesting: i32,
    pub t_colors: i32,
    pub include_none: i32,
    pub include_default: i32,
    pub include_link: i32,
    pub search_global: SearchState,
    pub need_check_timestamps: bool,
    pub did_check_timestamps: bool,
    pub no_check_timestamps: i32,
    pub mouse_grid: i32,
    pub mouse_row: i32,
    pub mouse_col: i32,
    pub mouse_past_bottom: bool,
    pub mouse_past_eol: bool,
    pub mouse_dragging: i32,
    pub root_menu: Vec<Vimmenu>,
    pub sys_menu: bool,
    pub firstwin: WinPtr,
    pub lastwin: WinPtr,
    pub prevwin: WinPtr,
    pub topframe: FramePtr,
    pub first_tabpage: TabpagePtr,
    pub curtab: TabpagePtr,
    pub lastused_tabpage: TabpagePtr,
    pub redraw_tabline: bool,
    pub firstbuf: BufPtr,
    pub lastbuf: BufPtr,
    pub curbuf: BufPtr,
    pub arg_had_last: bool,
    pub ru_col: i32,
    pub ru_wid: i32,
    pub sc_col: i32,
    pub exiting: bool,
    pub v_dying: i32,
    pub stdin_isatty: bool,
    pub stdout_isatty: bool,
    pub stderr_isatty: bool,
    pub stdin_fd: i32,
    pub full_screen: bool,
    pub secure: i32,
    pub textlock: i32,
    pub allbuf_lock: i32,
    pub sandbox: i32,
    pub silent_mode: bool,
    pub exmode_active: bool,
    pub visual_global: VisualState,
    pub where_paste_started: CursorPos,
    pub did_syncbind: bool,
    pub old_indent: i32,
    pub saved_cursor: CursorPos,
    pub ins_global: InsState,
    pub orig_line_count: Linenr,
    pub vr_lines_changed: i32,
    pub inhibit_delete_count: i32,
    pub fenc_default: Option<std::ffi::CString>,
    pub state_global: i32,
    pub debug_mode: bool,
    pub finish_op: bool,
    pub opcount: i32,
    pub motion_force: i32,
    pub cmdpreview: bool,
    pub reg_recording: i32,
    pub reg_executing: i32,
    pub pending_end_reg_executing: bool,
    pub reg_recorded: i32,
    pub no_mapping: i32,
    pub no_zero_mapping: i32,
    pub allow_keys: i32,
    pub no_u_sync: i32,
    pub u_sync_once: i32,
    pub force_restart_edit: bool,
    pub restart_edit: i32,
    pub ins_at_eol: bool,
    pub no_abbr: bool,
    pub mapped_ctrl_c: i32,
    pub ctrl_c_interrupts: bool,
    pub cmdmod: Cmdmod,
    pub msg_silent: i32,
    pub emsg_silent: i32,
    pub emsg_noredir: bool,
    pub cmd_silent: bool,
    pub in_assert_fails: bool,
    pub swap_exists_action: i32,
    pub swap_exists_did_quit: bool,
    pub iobuff: [u8; IOSIZE],
    pub name_buff: [u8; MAXPATHL],
    pub msg_buf: [u8; MSG_BUF_LEN],
    pub os_buf: [u8; MAXPATHL],
    pub redrawing_disabled: i32,
    pub readonlymode: bool,
    pub recoverymode: bool,
    pub typebuf: Typebuf,
    pub typebuf_was_empty: bool,
    pub ex_normal_busy: i32,
    pub expr_map_lock: i32,
    pub ignore_script: bool,
    pub key_typed: bool,
    pub key_stuffed: i32,
    pub maptick: i32,
    pub must_redraw: i32,
    pub skip_redraw: bool,
    pub do_redraw: bool,
    pub must_redraw_pum: bool,
    pub need_highlight_changed: bool,
    pub scriptout: Option<crate::os::fs::CFile>,
    pub got_int: bool,
    pub bangredo: bool,
    pub reg_do_extmatch: i32,
    pub re_extmatch_in: Option<std::rc::Rc<RegExtmatch>>,
    pub re_extmatch_out: Option<std::rc::Rc<RegExtmatch>>,
    pub did_outofmem_msg: bool,
    pub did_swapwrite_msg: bool,
    pub global_busy: i32,
    pub listcmd_busy: bool,
    pub need_start_insertmode: bool,
    pub last_mode: [u8; MODE_MAX_LENGTH],
    pub last_cmdline: Option<std::ffi::CString>,
    pub repeat_cmdline: Option<std::ffi::CString>,
    pub new_last_cmdline: Option<std::ffi::CString>,
    pub postponed_split: i32,
    pub postponed_split_flags: i32,
    pub postponed_split_tab: i32,
    pub g_do_tagpreview: i32,
    pub g_tag_at_cursor: bool,
    pub replace_offset: i32,
    pub keep_help_flag: bool,
    pub redir_off: bool,
    pub redir_fd: Option<crate::os::fs::CFile>,
    pub redir_reg: i32,
    pub redir_vname: bool,
    pub capture_ga: Option<Vec<u8>>,
    pub langmap_mapchar: [u8; 256],
    pub save_p_ls: i32,
    pub save_p_wmh: i32,
    pub wild_menu_showing: i32,
    pub globaldir: Option<std::ffi::CString>,
    pub last_chdir_reason: Option<&'static std::ffi::CStr>,
    pub km_stopsel: bool,
    pub km_startsel: bool,
    pub cmdwin_type: i32,
    pub cmdwin_buf: BufPtr,
    pub cmdline_win: WinPtr,
    pub sub_nsubs: i32,
    pub sub_nlines: Linenr,
    pub wim_flags: [u8; 4],
    pub stl_syntax: i32,
    pub typebuf_was_filled: bool,
    pub virtual_op: TriState,
    pub spell_redraw_lnum: Linenr,
    pub time_fd: Option<crate::os::fs::CFile>,
    pub vim_ignored: i32,
    pub embedded_mode: bool,
    pub headless_mode: bool,
    pub windows_version: [u8; 20],
    pub skip_win_fix_scroll: bool,
    pub skip_update_topline: bool,
    pub namespace_ids: Vec<(std::ffi::CString, i32)>,
    pub namespace_localscope: std::collections::BTreeSet<u32>,
    pub next_namespace_id: ObjHandle,
    pub buffer_handles: std::collections::BTreeMap<i32, BufPtr>,
    pub window_handles: std::collections::BTreeMap<i32, WinPtr>,
    pub tabpage_handles: std::collections::BTreeMap<i32, TabpagePtr>,
    pub last_cursormoved_win: WinPtr,
    pub last_cursormoved: CursorPos,
    pub autocmd_busy: bool,
    pub autocmd_no_enter: i32,
    pub autocmd_no_leave: i32,
    pub au_new_curbuf: Bufref,
    pub au_pending_free_buf: Vec<Box<FileBuf>>,
    pub au_pending_free_win: Vec<Box<WinInfoInfo>>,
    pub autocmd_fname: Option<std::ffi::CString>,
    pub autocmd_fname_full: bool,
    pub autocmd_bufnr: i32,
    pub autocmd_match: Option<std::ffi::CString>,
    pub did_cursorhold: bool,
    pub deferred_events: MultiQueue,
    pub channels: std::collections::BTreeMap<u64, Box<Channel>>,
    pub on_print: Callback,
    pub ctx_win_vec: Vec<CtxWin>,
    pub decor_state: DecorState,
    pub(crate) tv_arenas: crate::eval::typval::TypvalArenas,
    pub(crate) opt_vars: crate::option_vars::OptionVars,
    pub diff_context: i32,
    pub diff_foldcolumn: i32,
    pub diff_need_scrollbind: bool,
    pub need_diff_redraw: bool,
    pub win_extmark_arr: Vec<WinExtmark>,
    pub updating_screen: bool,
    pub redraw_not_allowed: bool,
    pub screen_search_hl: SynMatch,
    pub search_hl_has_cursor_lnum: Linenr,
    pub evalarg_evaluate: Evalarg,
    pub curbuf_splice_pending: i32,
    pub disable_fold_update: i32,
    pub default_grid: ScreenGrid,
    pub default_gridview: GridView,
    pub resizing_screen: bool,
    pub linebuf_char: Vec<Schar>,
    pub linebuf_attr: Vec<Sattr>,
    pub linebuf_vcol: Vec<Colnr>,
    pub highlight_attr: [i32; HLF_COUNT],
    pub highlight_attr_last: [i32; HLF_COUNT],
    pub highlight_user: [i32; 9],
    pub highlight_stlnc: [i32; 9],
    pub cterm_normal_fg_color: i32,
    pub cterm_normal_bg_color: i32,
    pub normal_fg: RgbValue,
    pub normal_bg: RgbValue,
    pub normal_sp: RgbValue,
    pub ns_hl_global: NS,
    pub ns_hl_win: NS,
    pub ns_hl_fast: NS,
    pub ns_hl_active: NS,
    pub hl_attr_active: Option<std::rc::Rc<crate::highlight::NSHlAttr>>,
    pub nlua_disable_preload: bool,
    pub tslua_query_parse_count: u64,
    pub repeat_luamap: i32,
    pub namedfm: [Xfmark; 36],
    pub arena_alloc_count: usize,
    pub msg_ext_skip_flush: bool,
    pub msg_ext_overwrite: bool,
    pub msg_ext_skip_verbose: bool,
    pub msg_ext_fast: bool,
    pub msg_grid: ScreenGrid,
    pub msg_grid_pos: i32,
    pub msg_grid_adj: GridView,
    pub msg_scrolled_at_flush: i32,
    pub msg_grid_scroll_discount: i32,
    pub msg_listdo_overwrite: i32,
    pub ch_before_blocking_events: MultiQueue,
    pub showcmd_buf: [u8; crate::normal::SHOWCMD_BUFLEN],
    pub nvim_testing: bool,
    pub pum_grid: ScreenGrid,
    pub pum_want: PumWant,
    pub last_insert_ga: Vec<u8>,
    pub add_last_insert: i32,
    pub tab_page_click_defs: Vec<StlClickDefinition>,
    pub ui_event_ns_id: u32,
    pub resize_events: MultiQueue,
    pub ui_refresh_cmdheight: bool,
    pub grid_line_buf_char: Vec<Schar>,
    pub grid_line_buf_attr: Vec<Sattr>,
    pub ui_client_channel_id: u64,
    pub ui_client_error_exit: i32,
    pub ui_client_exit_status: i32,
    pub ui_client_attached: bool,
    pub ui_client_forward_stdin: bool,
    pub tabpage_move_disallowed: i32,
}

impl EditorState {
    pub const fn new() -> Self {
        Self {
            main_loop: Loop::new(),
            g_stats: NvimStats {
                fsync: 0,
                redraw: 0,
                log_skip: 0,
            },
            rows_global: DFLT_ROWS,
            columns_global: DFLT_COLS,
            mod_mask: 0,
            vgetc_mod_mask: 0,
            vgetc_char: 0,
            cmdline_row: 0,
            redraw_cmdline: false,
            redraw_mode: false,
            clear_cmdline: false,
            mode_displayed: false,
            cmdline_star: 0,
            redrawing_cmdline: false,
            cmdline_was_last_drawn: false,
            exec_from_reg: false,
            dollar_vcol: -1,
            edit_submode: None,
            edit_submode_pre: None,
            edit_submode_extra: None,
            edit_submode_highl: 0,
            cmdmsg_rl: false,
            msg_col: 0,
            msg_row: 0,
            msg_scrolled: 0,
            msg_scrolled_ign: false,
            msg_did_scroll: false,
            keep_msg: None,
            keep_msg_hl_id: 0,
            need_fileinfo: false,
            msg_scroll: 0,
            msg_didout: false,
            msg_didany: false,
            msg_nowait: false,
            emsg_off: 0,
            info_message: false,
            msg_hist_off: false,
            need_clr_eos: false,
            emsg_skip: 0,
            emsg_severe: false,
            emsg_assert_fails_msg: None,
            emsg_assert_fails_lnum: 0,
            emsg_assert_fails_context: None,
            did_endif: false,
            did_emsg: 0,
            did_beep: 0,
            did_beep_silent: 0,
            did_emsg_syntax: false,
            called_emsg: 0,
            ex_exitval: 0,
            emsg_on_display: false,
            rc_did_emsg: false,
            no_wait_return: 0,
            need_wait_return: false,
            did_wait_return: false,
            need_maketitle: true,
            quit_more: false,
            vgetc_busy: 0,
            didset_vim: false,
            didset_vimruntime: false,
            lines_left: -1,
            msg_no_more: false,
            ex_nesting_level: 0,
            debug_break_level: -1,
            debug_did_msg: false,
            debug_tick: 0,
            debug_backtrace_level: 0,
            do_profiling: PROF_NONE,
            current_exception: None,
            did_throw: false,
            need_rethrow: false,
            check_cstack: false,
            trylevel: 0,
            force_abort: false,
            msg_list: None,
            suppress_errthrow: false,
            caught_stack: Vec::new(),
            may_garbage_collect: false,
            want_garbage_collect: false,
            garbage_collect_at_exit: false,
            current_sctx: Sctx {
                sc_sid: 0,
                sc_seq: 0,
                sc_lnum: 0,
                sc_chan: 0,
            },
            current_ui: 0,
            did_source_packages: false,
            provider_caller_scope: CallerScope {
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
            },
            provider_call_nesting: 0,
            t_colors: 256,
            include_none: 0,
            include_default: 0,
            include_link: 0,
            search_global: SearchState {
                hl_match: false,
                match_lines: 0,
                match_endcol: 0,
                first_line: 0,
                last_line: MAXLNUM,
                no_smartcase: false,
                cmdlen: 0,
                no_hlsearch: false,
            },
            need_check_timestamps: false,
            did_check_timestamps: false,
            no_check_timestamps: 0,
            mouse_grid: 0,
            mouse_row: 0,
            mouse_col: 0,
            mouse_past_bottom: false,
            mouse_past_eol: false,
            mouse_dragging: 0,
            root_menu: Vec::new(),
            sys_menu: false,
            firstwin: WinPtr::NULL,
            lastwin: WinPtr::NULL,
            prevwin: WinPtr::NULL,
            topframe: FramePtr::NULL,
            first_tabpage: TabpagePtr::NULL,
            curtab: TabpagePtr::NULL,
            lastused_tabpage: TabpagePtr::NULL,
            redraw_tabline: false,
            firstbuf: BufPtr::NULL,
            lastbuf: BufPtr::NULL,
            curbuf: BufPtr::NULL,
            arg_had_last: false,
            ru_col: 0,
            ru_wid: 0,
            sc_col: 0,
            exiting: false,
            v_dying: 0,
            stdin_isatty: true,
            stdout_isatty: true,
            stderr_isatty: true,
            stdin_fd: -1,
            full_screen: false,
            secure: 0,
            textlock: 0,
            allbuf_lock: 0,
            sandbox: 0,
            silent_mode: false,
            exmode_active: false,
            visual_global: VisualState {
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
            },
            where_paste_started: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            did_syncbind: false,
            old_indent: 0,
            saved_cursor: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            ins_global: InsState {
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
            },
            orig_line_count: 0,
            vr_lines_changed: 0,
            inhibit_delete_count: 0,
            fenc_default: None,
            state_global: MODE_NORMAL,
            debug_mode: false,
            finish_op: false,
            opcount: 0,
            motion_force: 0,
            cmdpreview: false,
            reg_recording: 0,
            reg_executing: 0,
            pending_end_reg_executing: false,
            reg_recorded: 0,
            no_mapping: 0,
            no_zero_mapping: 0,
            allow_keys: 0,
            no_u_sync: 0,
            u_sync_once: 0,
            force_restart_edit: false,
            restart_edit: 0,
            ins_at_eol: false,
            no_abbr: true,
            mapped_ctrl_c: 0,
            ctrl_c_interrupts: true,
            cmdmod: Cmdmod::new(),
            msg_silent: 0,
            emsg_silent: 0,
            emsg_noredir: false,
            cmd_silent: false,
            in_assert_fails: false,
            swap_exists_action: SEA_NONE,
            swap_exists_did_quit: false,
            iobuff: [0; IOSIZE],
            name_buff: [0; MAXPATHL],
            msg_buf: [0; MSG_BUF_LEN],
            os_buf: [0; MAXPATHL],
            redrawing_disabled: 0,
            readonlymode: false,
            recoverymode: false,
            typebuf: Typebuf::ZERO,
            typebuf_was_empty: false,
            ex_normal_busy: 0,
            expr_map_lock: 0,
            ignore_script: false,
            key_typed: false,
            key_stuffed: 0,
            maptick: 0,
            must_redraw: 0,
            skip_redraw: false,
            do_redraw: false,
            must_redraw_pum: false,
            need_highlight_changed: true,
            scriptout: None,
            got_int: false,
            bangredo: false,
            reg_do_extmatch: 0,
            re_extmatch_in: None,
            re_extmatch_out: None,
            did_outofmem_msg: false,
            did_swapwrite_msg: false,
            global_busy: 0,
            listcmd_busy: false,
            need_start_insertmode: false,
            last_mode: [b'n', 0, 0, 0],
            last_cmdline: None,
            repeat_cmdline: None,
            new_last_cmdline: None,
            postponed_split: 0,
            postponed_split_flags: 0,
            postponed_split_tab: 0,
            g_do_tagpreview: 0,
            g_tag_at_cursor: false,
            replace_offset: 0,
            keep_help_flag: false,
            redir_off: false,
            redir_fd: None,
            redir_reg: 0,
            redir_vname: false,
            capture_ga: None,
            langmap_mapchar: [0; 256],
            save_p_ls: -1,
            save_p_wmh: -1,
            wild_menu_showing: 0,
            globaldir: None,
            last_chdir_reason: None,
            km_stopsel: false,
            km_startsel: false,
            cmdwin_type: 0,
            cmdwin_buf: BufPtr::NULL,
            cmdline_win: WinPtr::NULL,
            sub_nsubs: 0,
            sub_nlines: 0,
            wim_flags: [0; 4],
            stl_syntax: 0,
            typebuf_was_filled: false,
            virtual_op: -1,
            spell_redraw_lnum: 0,
            time_fd: None,
            vim_ignored: 0,
            embedded_mode: false,
            headless_mode: false,
            windows_version: [0u8; 20],
            skip_win_fix_scroll: false,
            skip_update_topline: false,
            namespace_ids: Vec::new(),
            namespace_localscope: std::collections::BTreeSet::new(),
            next_namespace_id: 1,
            buffer_handles: std::collections::BTreeMap::new(),
            window_handles: std::collections::BTreeMap::new(),
            tabpage_handles: std::collections::BTreeMap::new(),
            last_cursormoved_win: WinPtr::NULL,
            last_cursormoved: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            autocmd_busy: false,
            autocmd_no_enter: 0,
            autocmd_no_leave: 0,
            au_new_curbuf: Bufref {
                br_buf: crate::globals::BufPtr::NULL,
                br_fnum: 0,
                br_buf_free_count: 0,
            },
            au_pending_free_buf: Vec::new(),
            au_pending_free_win: Vec::new(),
            autocmd_fname: None,
            autocmd_fname_full: false,
            autocmd_bufnr: 0,
            autocmd_match: None,
            did_cursorhold: true,
            deferred_events: MultiQueue::NULL,
            channels: std::collections::BTreeMap::new(),
            on_print: Callback::NONE,
            ctx_win_vec: Vec::new(),
            decor_state: DecorState::new(),
            tv_arenas: crate::eval::typval::TypvalArenas::new(),
            opt_vars: crate::option_vars::OptionVars::new(),
            diff_context: 6,
            diff_foldcolumn: 2,
            diff_need_scrollbind: false,
            need_diff_redraw: false,
            win_extmark_arr: Vec::new(),
            updating_screen: false,
            redraw_not_allowed: false,
            screen_search_hl: SynMatch {
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
            },
            search_hl_has_cursor_lnum: 0,
            evalarg_evaluate: Evalarg { eval_flags: 1 },
            curbuf_splice_pending: 0,
            disable_fold_update: 0,
            default_grid: ScreenGrid::with_target(GridTarget::Default),
            default_gridview: GridView {
                target: GridTarget::Default,
                row_offset: 0,
                col_offset: 0,
            },
            resizing_screen: false,
            linebuf_char: Vec::new(),
            linebuf_attr: Vec::new(),
            linebuf_vcol: Vec::new(),
            highlight_attr: [0; HLF_COUNT],
            highlight_attr_last: [0; HLF_COUNT],
            highlight_user: [0; 9],
            highlight_stlnc: [0; 9],
            cterm_normal_fg_color: 0,
            cterm_normal_bg_color: 0,
            normal_fg: -1,
            normal_bg: -1,
            normal_sp: -1,
            ns_hl_global: 0,
            ns_hl_win: -1,
            ns_hl_fast: -1,
            ns_hl_active: 0,
            hl_attr_active: None,
            nlua_disable_preload: false,
            tslua_query_parse_count: 0,
            repeat_luamap: 0,
            namedfm: [const { Xfmark::ZERO }; 36],
            arena_alloc_count: 0,
            msg_ext_skip_flush: false,
            msg_ext_overwrite: false,
            msg_ext_skip_verbose: false,
            msg_ext_fast: true,
            msg_grid: ScreenGrid::with_target(GridTarget::Msg),
            msg_grid_pos: 0,
            msg_grid_adj: GridView {
                target: GridTarget::None,
                row_offset: 0,
                col_offset: 0,
            },
            msg_scrolled_at_flush: 0,
            msg_grid_scroll_discount: 0,
            msg_listdo_overwrite: 0,
            ch_before_blocking_events: MultiQueue::NULL,
            showcmd_buf: [0; crate::normal::SHOWCMD_BUFLEN],
            nvim_testing: false,
            pum_grid: ScreenGrid::with_target(GridTarget::Pum),
            pum_want: PumWant {
                active: false,
                item: 0,
                insert: false,
                finish: false,
            },
            last_insert_ga: Vec::new(),
            add_last_insert: 0,
            tab_page_click_defs: Vec::new(),
            ui_event_ns_id: 0,
            resize_events: MultiQueue::NULL,
            ui_refresh_cmdheight: true,
            grid_line_buf_char: Vec::new(),
            grid_line_buf_attr: Vec::new(),
            ui_client_channel_id: 0,
            ui_client_error_exit: -1,
            ui_client_exit_status: 0,
            ui_client_attached: false,
            ui_client_forward_stdin: false,
            tabpage_move_disallowed: 0,
        }
    }

    #[inline(always)]
    pub const fn iobuff_bytes(&self) -> &[u8; IOSIZE] {
        &self.iobuff
    }

    #[inline(always)]
    pub const fn iobuff_bytes_mut(&mut self) -> &mut [u8; IOSIZE] {
        &mut self.iobuff
    }

    #[inline(always)]
    pub fn iobuff_cstr(&self) -> &'static std::ffi::CStr {
        std::ffi::CStr::from_bytes_until_nul(&EDITOR_STATE.handle().borrow().iobuff).unwrap_or(c"")
    }

    #[inline(always)]
    pub const fn name_buff_bytes_mut(&mut self) -> &mut [u8; MAXPATHL] {
        &mut self.name_buff
    }

    #[inline(always)]
    pub fn name_buff_cstr(&self) -> &'static std::ffi::CStr {
        std::ffi::CStr::from_bytes_until_nul(&EDITOR_STATE.handle().borrow().name_buff)
            .unwrap_or(c"")
    }

    #[inline(always)]
    pub const fn os_buf_bytes_mut(&mut self) -> &mut [u8; MAXPATHL] {
        &mut self.os_buf
    }

    #[inline(always)]
    pub const fn msg_buf_bytes_mut(&mut self) -> &mut [u8; MSG_BUF_LEN] {
        &mut self.msg_buf
    }
}

static EDITOR_STATE: GlobalState<EditorState> = GlobalState::new(EditorState::new());

#[inline(always)]
pub fn g_st() -> GraphPtr<EditorState> {
    EDITOR_STATE.handle()
}

#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
pub static curwin: std::sync::atomic::AtomicPtr<WinInfoInfo> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[inline(always)]
pub fn curwin_mut() -> &'static mut WinInfoInfo {
    curwin_ptr().borrow_mut()
}

#[inline(always)]
pub fn curwin_ptr() -> WinPtr {
    GraphPtr::new(curwin.load(std::sync::atomic::Ordering::Relaxed))
}

#[inline(always)]
pub fn set_curwin(wp: impl Into<WinPtr>) {
    curwin.store(wp.into().as_ptr(), std::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
pub fn curbuf_mut() -> &'static mut FileBuf {
    g_st().curbuf.borrow_mut()
}

#[inline(always)]
pub fn curbuf_opt_mut() -> Option<&'static mut FileBuf> {
    g_st().curbuf.opt_mut()
}

#[inline(always)]
pub fn curtab_mut() -> &'static mut TabPageInfo {
    g_st().curtab.borrow_mut()
}

#[inline(always)]
pub fn firstwin_mut() -> &'static mut WinInfoInfo {
    g_st().firstwin.borrow_mut()
}

#[inline(always)]
pub fn lastwin_mut() -> &'static mut WinInfoInfo {
    g_st().lastwin.borrow_mut()
}

#[inline(always)]
pub fn prevwin_opt() -> Option<&'static mut WinInfoInfo> {
    g_st().prevwin.opt_mut()
}

/// Zero-cost typed handle for a raw pointer to a global graph node (`WinInfoInfo`,
/// `FileBuf`, `TabPageInfo`, `Frame`, `TvList`, `TvDict`, etc.).
///
/// Holding a `GraphPtr<T>` across callbacks, autocommands, or event-loop ticks does not
/// assert LLVM `noalias` uniqueness (unlike `&mut T`), while providing checked `.opt()`,
/// `.opt_mut()`, `.borrow()`, and `.borrow_mut()` accessors for synchronous local borrows.
#[repr(transparent)]
pub struct GraphPtr<T>(*mut T);

impl<T> Copy for GraphPtr<T> {}

impl<T> Clone for GraphPtr<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Default for GraphPtr<T> {
    #[inline(always)]
    fn default() -> Self {
        Self::NULL
    }
}

impl<T> PartialEq for GraphPtr<T> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T> PartialEq<*mut T> for GraphPtr<T> {
    #[inline(always)]
    fn eq(&self, other: &*mut T) -> bool {
        self.as_ptr() == *other
    }
}

impl<T> PartialEq<*const T> for GraphPtr<T> {
    #[inline(always)]
    fn eq(&self, other: &*const T) -> bool {
        self.as_ptr().cast_const() == *other
    }
}

impl<T> PartialEq<&T> for GraphPtr<T> {
    #[inline(always)]
    fn eq(&self, other: &&T) -> bool {
        ptr::eq(self.as_ptr(), *other)
    }
}

impl<T> PartialEq<&mut T> for GraphPtr<T> {
    #[inline(always)]
    fn eq(&self, other: &&mut T) -> bool {
        ptr::eq(self.as_ptr(), *other)
    }
}

macro_rules! impl_sym_graph_eq {
    ($($t:ty),* $(,)?) => {
        $(
            impl PartialEq<GraphPtr<$t>> for *mut $t {
                #[inline(always)]
                fn eq(&self, other: &GraphPtr<$t>) -> bool {
                    *self == other.as_ptr()
                }
            }
            impl PartialEq<GraphPtr<$t>> for *const $t {
                #[inline(always)]
                fn eq(&self, other: &GraphPtr<$t>) -> bool {
                    *self == other.as_ptr().cast_const()
                }
            }
            impl PartialEq<GraphPtr<$t>> for &$t {
                #[inline(always)]
                fn eq(&self, other: &GraphPtr<$t>) -> bool {
                    ptr::eq(*self, other.as_ptr())
                }
            }
            impl PartialEq<GraphPtr<$t>> for &mut $t {
                #[inline(always)]
                fn eq(&self, other: &GraphPtr<$t>) -> bool {
                    ptr::eq(*self, other.as_ptr())
                }
            }
        )*
    };
}

impl_sym_graph_eq!(WinInfoInfo, FileBuf, TabPageInfo, Frame);

impl<T> Eq for GraphPtr<T> {}

impl<T> PartialOrd for GraphPtr<T> {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for GraphPtr<T> {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.0 as usize).cmp(&(other.0 as usize))
    }
}

impl<T> std::fmt::Debug for GraphPtr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Pointer::fmt(&self.as_ptr(), f)
    }
}

impl<T> From<*mut T> for GraphPtr<T> {
    #[inline(always)]
    fn from(ptr: *mut T) -> Self {
        Self::new(ptr)
    }
}

impl<T> From<&T> for GraphPtr<T> {
    #[inline(always)]
    fn from(r: &T) -> Self {
        Self::new(std::hint::black_box(&raw const *r).cast_mut())
    }
}

impl<T> From<&mut T> for GraphPtr<T> {
    #[inline(always)]
    fn from(r: &mut T) -> Self {
        Self::new(std::hint::black_box(&raw mut *r))
    }
}

impl<T> From<Option<&T>> for GraphPtr<T> {
    #[inline(always)]
    fn from(opt: Option<&T>) -> Self {
        match opt {
            Some(r) => Self::new(std::hint::black_box(&raw const *r).cast_mut()),
            None => Self::NULL,
        }
    }
}

impl<T> From<Option<&mut T>> for GraphPtr<T> {
    #[inline(always)]
    fn from(opt: Option<&mut T>) -> Self {
        match opt {
            Some(r) => Self::new(std::hint::black_box(&raw mut *r)),
            None => Self::NULL,
        }
    }
}

impl<T> std::ops::Deref for GraphPtr<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        unsafe { &*self.0 }
    }
}

impl<T> std::ops::DerefMut for GraphPtr<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.0 }
    }
}

pub trait AsGraphPtr<T> {
    fn into_graph_ptr(self) -> GraphPtr<T>;
}

impl<T> AsGraphPtr<T> for GraphPtr<T> {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        self
    }
}

impl<T> AsGraphPtr<T> for &GraphPtr<T> {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        *self
    }
}

impl<T> AsGraphPtr<T> for &mut GraphPtr<T> {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        *self
    }
}

impl<T> AsGraphPtr<T> for &T {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::from(self)
    }
}

impl<T> AsGraphPtr<T> for &&T {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::from(*self)
    }
}

impl<T> AsGraphPtr<T> for &mut T {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::from(&*self)
    }
}

impl<T> AsGraphPtr<T> for &&mut T {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::from(&**self)
    }
}

impl<T> AsGraphPtr<T> for Option<&T> {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::from(self)
    }
}

impl<T> AsGraphPtr<T> for Option<&mut T> {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::from(self)
    }
}

impl<T> AsGraphPtr<T> for &Option<&T> {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::from(*self)
    }
}

impl<T> AsGraphPtr<T> for &Option<&mut T> {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::from(self.as_deref())
    }
}

impl<T> AsGraphPtr<T> for *mut T {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::new(self)
    }
}

impl<T> AsGraphPtr<T> for &*mut T {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::new(*self)
    }
}

impl<T> AsGraphPtr<T> for *const T {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::new(self.cast_mut())
    }
}

impl<T> AsGraphPtr<T> for &*const T {
    #[inline(always)]
    fn into_graph_ptr(self) -> GraphPtr<T> {
        GraphPtr::new((*self).cast_mut())
    }
}

impl<T> GraphPtr<T> {
    pub const NULL: Self = Self(ptr::null_mut());

    #[inline(always)]
    pub const fn new(ptr: *mut T) -> Self {
        Self(ptr)
    }

    #[inline(always)]
    pub fn of(val: impl AsGraphPtr<T>) -> Self {
        val.into_graph_ptr()
    }

    #[inline(always)]
    pub fn from_ref(r: &T) -> Self {
        Self::from(r)
    }

    #[inline(always)]
    pub const fn as_ptr(self) -> *mut T {
        self.0
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0.is_null()
    }

    #[inline(always)]
    pub const fn cast<U>(self) -> *mut U {
        self.0.cast()
    }

    #[inline(always)]
    pub const fn cast_ptr<U>(self) -> GraphPtr<U> {
        GraphPtr(self.0.cast())
    }

    #[inline(always)]
    pub fn as_ref<'a>(self) -> Option<&'a T> {
        self.opt()
    }

    #[allow(dead_code)]
    #[inline(always)]
    pub fn as_mut<'a>(self) -> Option<&'a mut T> {
        self.opt_mut()
    }

    #[inline(always)]
    pub fn opt<'a>(self) -> Option<&'a T> {
        if (self.0 as usize) == 0 {
            None
        } else {
            Some(unsafe { &*self.0 })
        }
    }

    #[inline(always)]
    pub fn opt_mut<'a>(self) -> Option<&'a mut T> {
        if (self.0 as usize) == 0 {
            None
        } else {
            Some(unsafe { &mut *self.0 })
        }
    }

    #[inline(always)]
    pub const fn borrow<'a>(self) -> &'a T {
        unsafe { &*self.0 }
    }

    #[inline(always)]
    pub const fn borrow_mut<'a>(self) -> &'a mut T {
        unsafe { &mut *self.0 }
    }

    #[inline(always)]
    pub const fn wrapping_add(self, count: usize) -> Self {
        Self::new(self.as_ptr().wrapping_add(count))
    }

    #[inline(always)]
    pub const fn wrapping_sub(self, count: usize) -> Self {
        Self::new(self.as_ptr().wrapping_sub(count))
    }

    #[inline(always)]
    pub const fn wrapping_offset(self, count: isize) -> Self {
        Self::new(self.as_ptr().wrapping_offset(count))
    }

    #[inline(always)]
    pub const fn add(self, count: usize) -> Self {
        self.wrapping_add(count)
    }

    #[inline(always)]
    pub const fn sub(self, count: usize) -> Self {
        self.wrapping_sub(count)
    }

    #[inline(always)]
    pub const fn offset(self, count: isize) -> Self {
        self.wrapping_offset(count)
    }

    #[inline(always)]
    pub fn offset_from(self, origin: impl AsGraphPtr<T>) -> isize {
        unsafe { self.0.offset_from(origin.into_graph_ptr().as_ptr()) }
    }

    #[inline(always)]
    pub const fn cast_const(self) -> Self {
        self
    }

    #[inline(always)]
    pub const fn cast_mut(self) -> Self {
        self
    }

    #[inline(always)]
    pub fn addr(self) -> usize {
        self.0 as usize
    }

    #[inline(always)]
    pub fn into_box(self) -> Box<T> {
        unsafe { Box::from_raw(self.0) }
    }

    #[inline(always)]
    pub fn drop_box(self) {
        if !self.0.is_null() {
            unsafe {
                drop(Box::from_raw(self.0));
            }
        }
    }

    #[inline(always)]
    pub fn replace_val(self, src: T) -> T {
        unsafe { ptr::replace(self.0, src) }
    }
}

pub type WinPtr = GraphPtr<WinInfoInfo>;
pub type BufPtr = GraphPtr<FileBuf>;
pub type TabpagePtr = GraphPtr<TabPageInfo>;
pub type FramePtr = GraphPtr<Frame>;

#[inline(always)]
pub fn curwin_handle() -> WinPtr {
    curwin_ptr()
}

#[inline(always)]
pub fn curtab_handle() -> TabpagePtr {
    g_st().curtab
}

pub struct BufMutIter(BufPtr);

impl Iterator for BufMutIter {
    type Item = &'static mut FileBuf;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.b_next;
        Some(cur)
    }
}

pub struct BufPtrIter(BufPtr);

impl Iterator for BufPtrIter {
    type Item = &'static mut FileBuf;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.b_next;
        Some(cur)
    }
}

#[inline(always)]
pub fn for_all_buffers_mut() -> BufMutIter {
    BufMutIter(g_st().firstbuf)
}

#[inline(always)]
pub fn buf_iter_mut(start: impl Into<BufPtr>) -> BufMutIter {
    BufMutIter(start.into())
}

#[inline(always)]
pub fn for_all_buffers_ptr() -> BufPtrIter {
    BufPtrIter(g_st().firstbuf)
}

pub struct BufRevPtrIter(BufPtr);

impl Iterator for BufRevPtrIter {
    type Item = &'static mut FileBuf;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.b_prev;
        Some(cur)
    }
}

#[inline(always)]
pub fn for_all_buffers_rev_ptr() -> BufRevPtrIter {
    BufRevPtrIter(g_st().lastbuf)
}

pub struct WinMutIter(WinPtr);

impl Iterator for WinMutIter {
    type Item = &'static mut WinInfoInfo;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.w_next;
        Some(cur)
    }
}

pub struct WinPtrIter(WinPtr);

impl Iterator for WinPtrIter {
    type Item = &'static mut WinInfoInfo;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.w_next;
        Some(cur)
    }
}

#[inline(always)]
pub fn for_all_windows_mut() -> WinMutIter {
    WinMutIter(g_st().firstwin)
}

#[inline(always)]
pub fn win_iter_mut(start: impl Into<WinPtr>) -> WinMutIter {
    WinMutIter(start.into())
}

#[inline(always)]
pub fn for_all_windows_in_tab_mut(tp: &TabPageInfo) -> WinMutIter {
    WinMutIter(if g_st().curtab == tp {
        g_st().firstwin
    } else {
        tp.tp_firstwin
    })
}

#[inline(always)]
pub fn for_all_windows_ptr() -> WinPtrIter {
    WinPtrIter(g_st().firstwin)
}

#[inline(always)]
pub fn win_iter_ptr(start: impl Into<WinPtr>) -> WinPtrIter {
    WinPtrIter(start.into())
}

#[inline(always)]
pub fn for_all_windows_in_tab_ptr(tp: &TabPageInfo) -> WinPtrIter {
    WinPtrIter(if g_st().curtab == tp {
        g_st().firstwin
    } else {
        tp.tp_firstwin
    })
}

pub struct WinRevPtrIter(WinPtr);

impl Iterator for WinRevPtrIter {
    type Item = &'static mut WinInfoInfo;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.w_prev;
        Some(cur)
    }
}

#[inline(always)]
pub fn for_all_windows_rev_ptr() -> WinRevPtrIter {
    WinRevPtrIter(g_st().lastwin)
}

pub struct TabMutIter(TabpagePtr);

impl Iterator for TabMutIter {
    type Item = &'static mut TabPageInfo;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.tp_next;
        Some(cur)
    }
}

pub struct TabPtrIter(TabpagePtr);

impl Iterator for TabPtrIter {
    type Item = &'static mut TabPageInfo;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.tp_next;
        Some(cur)
    }
}

#[inline(always)]
pub fn for_all_tabpages_mut() -> TabMutIter {
    TabMutIter(g_st().first_tabpage)
}

#[inline(always)]
pub fn for_all_tabpages_ptr() -> TabPtrIter {
    TabPtrIter(g_st().first_tabpage)
}

pub struct FrameMutIter(FramePtr);

impl Iterator for FrameMutIter {
    type Item = &'static mut Frame;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.fr_next;
        Some(cur)
    }
}

pub struct FramePtrIter(FramePtr);

impl Iterator for FramePtrIter {
    type Item = &'static mut Frame;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.fr_next;
        Some(cur)
    }
}

#[inline(always)]
pub fn frame_iter_mut(start: FramePtr) -> FrameMutIter {
    FrameMutIter(start)
}

#[inline(always)]
pub fn frame_iter_ptr(start: FramePtr) -> FramePtrIter {
    FramePtrIter(start)
}

#[allow(non_upper_case_globals)]
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
#[allow(non_upper_case_globals)]
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
    g_st().time_fd.is_some()
}
#[inline(always)]
pub fn has_redir_fd() -> bool {
    g_st().redir_fd.is_some()
}
#[inline(always)]
pub fn has_scriptout() -> bool {
    g_st().scriptout.is_some()
}
pub const SCREEN_GRID_INIT: ScreenGrid = ScreenGrid {
    handle: 0,
    target_kind: GridTarget::None,
    chars: Vec::new(),
    attrs: Vec::new(),
    vcols: Vec::new(),
    line_offset: Vec::new(),
    dirty_col: Vec::new(),
    rows: 0,
    cols: 0,
    valid: false,
    throttled: false,
    blending: false,
    mouse_enabled: true,
    zindex: 0,
    comp_row: 0,
    comp_col: 0,
    comp_width: 0,
    comp_height: 0,
    comp_index: 0,
    comp_disabled: false,
    pending_comp_index_update: true,
};
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
#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
pub static test_disable_char_avail: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
pub const FLOAT_ANCHOR_STR: [&std::ffi::CStr; 4] = [c"NW", c"NE", c"SW", c"SE"];

#[cfg(test)]
pub(crate) static TEST_GLOBAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[inline(always)]
pub fn topframe_mut() -> &'static mut Frame {
    g_st().topframe.borrow_mut()
}

#[inline(always)]
pub fn first_tabpage_mut() -> &'static mut TabPageInfo {
    g_st().first_tabpage.borrow_mut()
}

#[inline(always)]
pub fn prevwin_mut() -> &'static mut WinInfoInfo {
    g_st().prevwin.borrow_mut()
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
            assert_eq!({ g_st().rows_global }, 24);
            assert_eq!({ g_st().columns_global }, 80);
            assert_eq!({ g_st().dollar_vcol }, -1);
            assert_eq!({ g_st().t_colors }, 256);
            assert_eq!(starting_val(), 2);
            assert_eq!({ g_st().state_global }, 0x01);
            assert_eq!({ g_st().visual_global.mode }, b'v' as i32);
            assert_eq!({ g_st().search_global.last_line }, 0x7fffffff);
            assert_eq!(NO_LINES_MSG.to_bytes(), b"--No lines in buffer--");
        }
    }
}
