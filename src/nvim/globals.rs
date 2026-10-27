//! Global variables (`src/nvim/globals.h`).

use crate::highlight::{HLF_COUNT, RgbValue};
use std::ffi::{c_char, c_int, c_long};
use std::ptr;

use crate::decoration::{DecorSignHighlight, DecorState, NS};
use crate::ex_docmd::cmdmod_T;

use crate::api::ui::NO_SCREEN;
use crate::arglist::alist_T;
use crate::buffer::{MAXLNUM, SEA_NONE, buf_T, bufref_T};
use crate::channel::Channel;
use crate::context::CtxWin;
use crate::cursor::{colnr_T, linenr_T, pos_T};
use crate::drawscreen::WinExtmark;
use crate::eval::caller_scope;
use crate::eval::typval::Callback;
use crate::eval::userfunc::MSG_BUF_LEN;
use crate::eval::vars::evalarg_T;
use crate::event::multiqueue::MultiQueue;
use crate::ex_eval::{except_T, msglist_T};
use crate::ex_getln::{disptick_T, hlf_T};
use crate::extmark::TriState;
use crate::grid::{GridView, ScreenGrid, sattr_T, schar_T};
use crate::lua::converter::nlua_ref_state_t;
use crate::mapping::typebuf_T;
use crate::mark::xfmark_T;
use crate::r#match::{SearchState, match_T};
use crate::menu::vimmenu_T;
use crate::mouse::StlClickDefinition;
use crate::msgpack_rpc::packer::handle_T;
use crate::normal::VisualState;
use crate::os::fs::{MAXPATHL, NvimStats};
use crate::popupmenu::PumWant;
use crate::regexp::reg_extmatch_T;
use crate::runtime::sctx_T;
use crate::state::{MODE_MAX_LENGTH, MODE_NORMAL};
use crate::textformat::InsState;
use crate::window::{frame_T, tabpage_T, win_T};

pub(crate) const IOSIZE: usize = 1024 + 1;
pub(crate) const DFLT_ROWS: c_int = 24;
pub(crate) const DFLT_COLS: c_int = 80;
pub(crate) const PROF_NONE: c_int = 0;

use crate::errors::bytes_to_chars;
use crate::event::r#loop::Loop;

#[repr(transparent)]
pub struct GlobalState<T>(std::cell::UnsafeCell<T>);

unsafe impl<T> Sync for GlobalState<T> {}

impl<T> GlobalState<T> {
    #[inline]
    pub const fn new(val: T) -> Self {
        Self(std::cell::UnsafeCell::new(val))
    }

    #[inline]
    pub const fn as_ptr(&self) -> *mut T {
        self.0.get()
    }
}

pub static mut main_loop: Loop = Loop::new();

pub static mut g_stats: NvimStats = NvimStats {
    fsync: 0,
    redraw: 0,
    log_skip: 0,
};
pub static mut Rows: c_int = DFLT_ROWS;
pub static mut Columns: c_int = DFLT_COLS;
pub static mut mod_mask: c_int = 0;
pub static mut vgetc_mod_mask: c_int = 0;
pub static mut vgetc_char: c_int = 0;
pub static mut cmdline_row: c_int = 0;
pub static mut redraw_cmdline: bool = false;
pub static mut redraw_mode: bool = false;
pub static mut clear_cmdline: bool = false;
pub static mut mode_displayed: bool = false;
pub static mut cmdline_star: c_int = 0;
pub static mut redrawing_cmdline: bool = false;
pub static mut cmdline_was_last_drawn: bool = false;
pub static mut exec_from_reg: bool = false;
pub static mut dollar_vcol: colnr_T = -1;
pub static mut edit_submode: *const c_char = ptr::null();
pub static mut edit_submode_pre: *const c_char = ptr::null();
pub static mut edit_submode_extra: *mut c_char = ptr::null_mut();
pub static mut edit_submode_highl: hlf_T = 0;
pub static mut cmdmsg_rl: bool = false;
pub static mut msg_col: c_int = 0;
pub static mut msg_row: c_int = 0;
pub static mut msg_scrolled: c_int = 0;
pub static mut msg_scrolled_ign: bool = false;
pub static mut msg_did_scroll: bool = false;
pub static mut keep_msg: Option<std::ffi::CString> = None;
pub static mut keep_msg_hl_id: c_int = 0;
pub static mut need_fileinfo: bool = false;
pub static mut msg_scroll: c_int = 0;
pub static mut msg_didout: bool = false;
pub static mut msg_didany: bool = false;
pub static mut msg_nowait: bool = false;
pub static mut emsg_off: c_int = 0;
pub static mut info_message: bool = false;
pub static mut msg_hist_off: bool = false;
pub static mut need_clr_eos: bool = false;
pub static mut emsg_skip: c_int = 0;
pub static mut emsg_severe: bool = false;
pub static mut emsg_assert_fails_msg: Option<std::ffi::CString> = None;
pub static mut emsg_assert_fails_lnum: c_long = 0;
pub static mut emsg_assert_fails_context: Option<std::ffi::CString> = None;
pub static mut did_endif: bool = false;
pub static mut did_emsg: c_int = 0;
pub static mut did_beep: u64 = 0;
pub static mut did_beep_silent: u64 = 0;
pub static mut did_emsg_syntax: bool = false;
pub static mut called_emsg: c_int = 0;
pub static mut ex_exitval: c_int = 0;
pub static mut emsg_on_display: bool = false;
pub static mut rc_did_emsg: bool = false;
pub static mut no_wait_return: c_int = 0;
pub static mut need_wait_return: bool = false;
pub static mut did_wait_return: bool = false;
pub static mut need_maketitle: bool = true;
pub static mut quit_more: bool = false;
pub static mut vgetc_busy: c_int = 0;
pub static mut didset_vim: bool = false;
pub static mut didset_vimruntime: bool = false;
pub static mut lines_left: c_int = -1;
pub static mut msg_no_more: bool = false;
pub static mut ex_nesting_level: c_int = 0;
pub static mut debug_break_level: c_int = -1;
pub static mut debug_did_msg: bool = false;
pub static mut debug_tick: c_int = 0;
pub static mut debug_backtrace_level: c_int = 0;
pub static mut do_profiling: c_int = PROF_NONE;
pub static mut current_exception: Option<std::rc::Rc<except_T>> = None;
pub static mut did_throw: bool = false;
pub static mut need_rethrow: bool = false;
pub static mut check_cstack: bool = false;
pub static mut trylevel: c_int = 0;
pub static mut force_abort: bool = false;
pub static mut msg_list: Option<Vec<msglist_T>> = None;
pub static mut suppress_errthrow: bool = false;
pub static mut caught_stack: Vec<std::rc::Rc<except_T>> = Vec::new();
pub static mut may_garbage_collect: bool = false;
pub static mut want_garbage_collect: bool = false;
pub static mut garbage_collect_at_exit: bool = false;
pub static mut current_sctx: sctx_T = sctx_T {
    sc_sid: 0,
    sc_seq: 0,
    sc_lnum: 0,
    sc_chan: 0,
};
pub static mut current_ui: u64 = 0;
pub static mut did_source_packages: bool = false;
pub static mut provider_caller_scope: caller_scope = caller_scope {
    script_ctx: sctx_T {
        sc_sid: 0,
        sc_seq: 0,
        sc_lnum: 0,
        sc_chan: 0,
    },
    es_entry: crate::profile::estack_T {
        es_lnum: 0,
        es_name: None,
        es_type: 0,
        es_info: crate::profile::EStackInfo::None,
    },
    autocmd_fname: None,
    autocmd_match: None,
    autocmd_fname_full: false,
    autocmd_bufnr: 0,
    funccalp: ptr::null_mut(),
};
pub static mut provider_call_nesting: c_int = 0;
pub static mut t_colors: c_int = 256;
pub static mut include_none: c_int = 0;
pub static mut include_default: c_int = 0;
pub static mut include_link: c_int = 0;
pub static mut Search: SearchState = SearchState {
    hl_match: false,
    match_lines: 0,
    match_endcol: 0,
    first_line: 0,
    last_line: MAXLNUM,
    no_smartcase: false,
    cmdlen: 0,
    no_hlsearch: false,
};
pub static mut need_check_timestamps: bool = false;
pub static mut did_check_timestamps: bool = false;
pub static mut no_check_timestamps: c_int = 0;
pub static mut mouse_grid: c_int = 0;
pub static mut mouse_row: c_int = 0;
pub static mut mouse_col: c_int = 0;
pub static mut mouse_past_bottom: bool = false;
pub static mut mouse_past_eol: bool = false;
pub static mut mouse_dragging: c_int = 0;
pub static mut root_menu: Vec<vimmenu_T> = Vec::new();
pub static mut sys_menu: bool = false;
pub static mut firstwin: *mut win_T = ptr::null_mut();
pub static mut lastwin: *mut win_T = ptr::null_mut();
pub static mut prevwin: *mut win_T = ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut curwin: *mut win_T = ptr::null_mut();
pub static mut topframe: *mut frame_T = ptr::null_mut();
pub static mut first_tabpage: *mut tabpage_T = ptr::null_mut();
pub static mut curtab: *mut tabpage_T = ptr::null_mut();
pub static mut lastused_tabpage: *mut tabpage_T = ptr::null_mut();
pub static mut redraw_tabline: bool = false;
pub static mut firstbuf: *mut buf_T = ptr::null_mut();
pub static mut lastbuf: *mut buf_T = ptr::null_mut();
pub static mut curbuf: *mut buf_T = ptr::null_mut();
pub static mut global_alist: alist_T = alist_T::new();
pub static mut max_alist_id: c_int = 0;
pub static mut arg_had_last: bool = false;
pub static mut ru_col: c_int = 0;
pub static mut ru_wid: c_int = 0;
pub static mut sc_col: c_int = 0;
#[unsafe(no_mangle)]
pub static mut starting: c_int = NO_SCREEN;
pub static mut exiting: bool = false;
pub static mut v_dying: c_int = 0;
pub static mut stdin_isatty: bool = true;
pub static mut stdout_isatty: bool = true;
pub static mut stderr_isatty: bool = true;
pub static mut stdin_fd: c_int = -1;
pub static mut full_screen: bool = false;
pub static mut secure: c_int = 0;
pub static mut textlock: c_int = 0;
pub static mut allbuf_lock: c_int = 0;
pub static mut sandbox: c_int = 0;
pub static mut silent_mode: bool = false;
pub static mut exmode_active: bool = false;
pub static mut Visual: VisualState = VisualState {
    start: pos_T {
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
    mode: b'v' as c_int,
    resel: crate::normal::VisualExtent {
        mode: 0,
        line_count: 0,
        vcol: 0,
    },
};
pub static mut where_paste_started: pos_T = pos_T {
    lnum: 0,
    col: 0,
    coladd: 0,
};
pub static mut did_syncbind: bool = false;
pub static mut old_indent: c_int = 0;
pub static mut saved_cursor: pos_T = pos_T {
    lnum: 0,
    col: 0,
    coladd: 0,
};
pub static mut Ins: InsState = InsState {
    start: pos_T {
        lnum: 0,
        col: 0,
        coladd: 0,
    },
    start_orig: pos_T {
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
    dont_sync_undo: crate::extmark::kNone,
    o_lnum: 0,
};
pub static mut orig_line_count: linenr_T = 0;
pub static mut vr_lines_changed: c_int = 0;
pub static mut inhibit_delete_count: c_int = 0;
pub static mut fenc_default: Option<std::ffi::CString> = None;
pub static mut State: c_int = MODE_NORMAL;
pub static mut debug_mode: bool = false;
pub static mut finish_op: bool = false;
pub static mut opcount: c_int = 0;
pub static mut motion_force: c_int = 0;
pub static mut cmdpreview: bool = false;
pub static mut reg_recording: c_int = 0;
pub static mut reg_executing: c_int = 0;
pub static mut pending_end_reg_executing: bool = false;
pub static mut reg_recorded: c_int = 0;
pub static mut no_mapping: c_int = 0;
pub static mut no_zero_mapping: c_int = 0;
pub static mut allow_keys: c_int = 0;
pub static mut no_u_sync: c_int = 0;
pub static mut u_sync_once: c_int = 0;
pub static mut force_restart_edit: bool = false;
pub static mut restart_edit: c_int = 0;
pub static mut ins_at_eol: bool = false;
pub static mut no_abbr: bool = true;
pub static mut mapped_ctrl_c: c_int = 0;
pub static mut ctrl_c_interrupts: bool = true;
pub static mut cmdmod: cmdmod_T = cmdmod_T::new();
pub static mut msg_silent: c_int = 0;
pub static mut emsg_silent: c_int = 0;
pub static mut emsg_noredir: bool = false;
pub static mut cmd_silent: bool = false;
pub static mut in_assert_fails: bool = false;
pub static mut swap_exists_action: c_int = SEA_NONE;
pub static mut swap_exists_did_quit: bool = false;
pub static mut IObuff: [c_char; IOSIZE] = [0 as c_char; IOSIZE];
pub static mut NameBuff: [c_char; MAXPATHL] = [0 as c_char; MAXPATHL];
pub static mut msg_buf: [c_char; MSG_BUF_LEN] = [0 as c_char; MSG_BUF_LEN];
pub static mut os_buf: [c_char; MAXPATHL] = [0 as c_char; MAXPATHL];
pub static mut RedrawingDisabled: c_int = 0;
pub static mut readonlymode: bool = false;
pub static mut recoverymode: bool = false;
pub static mut typebuf: typebuf_T = typebuf_T::ZERO;
pub static mut typebuf_was_empty: bool = false;
pub static mut ex_normal_busy: c_int = 0;
pub static mut expr_map_lock: c_int = 0;
pub static mut ignore_script: bool = false;
pub static mut KeyTyped: bool = false;
pub static mut KeyStuffed: c_int = 0;
pub static mut maptick: c_int = 0;
pub static mut must_redraw: c_int = 0;
pub static mut skip_redraw: bool = false;
pub static mut do_redraw: bool = false;
pub static mut must_redraw_pum: bool = false;
pub static mut need_highlight_changed: bool = true;
pub static mut scriptout: Option<crate::os::fs::CFile> = None;
pub static mut got_int: bool = false;
pub static mut bangredo: bool = false;
pub static mut reg_do_extmatch: c_int = 0;
pub static mut re_extmatch_in: Option<std::rc::Rc<reg_extmatch_T>> = None;
pub static mut re_extmatch_out: Option<std::rc::Rc<reg_extmatch_T>> = None;
pub static mut did_outofmem_msg: bool = false;
pub static mut did_swapwrite_msg: bool = false;
pub static mut global_busy: c_int = 0;
pub static mut listcmd_busy: bool = false;
pub static mut need_start_insertmode: bool = false;
pub static mut last_mode: [c_char; MODE_MAX_LENGTH] = [b'n' as c_char, 0, 0, 0];
pub static mut last_cmdline: Option<std::ffi::CString> = None;
pub static mut repeat_cmdline: Option<std::ffi::CString> = None;
pub static mut new_last_cmdline: Option<std::ffi::CString> = None;
pub static mut postponed_split: c_int = 0;
pub static mut postponed_split_flags: c_int = 0;
pub static mut postponed_split_tab: c_int = 0;
pub static mut g_do_tagpreview: c_int = 0;
pub static mut g_tag_at_cursor: bool = false;
pub static mut replace_offset: c_int = 0;
pub const escape_chars: &std::ffi::CStr = c" \t\\\"|";
pub static mut keep_help_flag: bool = false;
pub static mut redir_off: bool = false;
pub static mut redir_fd: Option<crate::os::fs::CFile> = None;
pub static mut redir_reg: c_int = 0;
pub static mut redir_vname: bool = false;
pub static mut capture_ga: *mut Vec<c_char> = ptr::null_mut();
pub static mut langmap_mapchar: [u8; 256] = [0; 256];
pub static mut save_p_ls: c_int = -1;
pub static mut save_p_wmh: c_int = -1;
pub static mut wild_menu_showing: c_int = 0;
pub static mut globaldir: Option<std::ffi::CString> = None;
pub static mut last_chdir_reason: Option<&'static std::ffi::CStr> = None;
pub static mut km_stopsel: bool = false;
pub static mut km_startsel: bool = false;
pub static mut cmdwin_type: c_int = 0;
pub static mut cmdwin_buf: *mut buf_T = ptr::null_mut();
pub static mut cmdline_win: *mut win_T = ptr::null_mut();
pub static no_lines_msg: [c_char; 23] = bytes_to_chars(b"--No lines in buffer--\0");
pub static mut sub_nsubs: c_int = 0;
pub static mut sub_nlines: linenr_T = 0;
pub static mut wim_flags: [u8; 4] = [0; 4];
pub static mut stl_syntax: c_int = 0;
pub static mut typebuf_was_filled: bool = false;
pub static mut virtual_op: TriState = -1;
#[unsafe(no_mangle)]
pub static mut display_tick: disptick_T = 0;
pub static mut spell_redraw_lnum: linenr_T = 0;
pub static mut time_fd: Option<crate::os::fs::CFile> = None;

#[inline]
pub fn has_time_fd() -> bool {
    unsafe { (*(&raw const time_fd)).is_some() }
}

#[inline]
pub fn has_redir_fd() -> bool {
    unsafe { (*(&raw const redir_fd)).is_some() }
}

#[inline]
pub fn has_scriptout() -> bool {
    unsafe { (*(&raw const scriptout)).is_some() }
}
pub static mut vim_ignored: c_int = 0;
pub static mut embedded_mode: bool = false;
pub static mut headless_mode: bool = false;
pub static mut windowsVersion: [c_char; 20] = [0 as c_char; 20];
pub static mut skip_win_fix_scroll: bool = false;
pub static mut skip_update_topline: bool = false;

pub const SCREEN_GRID_INIT: ScreenGrid = ScreenGrid {
    handle: 0,
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

pub static mut namespace_ids: Vec<(std::ffi::CString, c_int)> = Vec::new();
pub static mut namespace_localscope: std::collections::BTreeSet<u32> =
    std::collections::BTreeSet::new();
pub static mut next_namespace_id: handle_T = 1;
pub static mut buffer_handles: std::collections::BTreeMap<c_int, *mut buf_T> =
    std::collections::BTreeMap::new();
pub static mut window_handles: std::collections::BTreeMap<c_int, *mut win_T> =
    std::collections::BTreeMap::new();
pub static mut tabpage_handles: std::collections::BTreeMap<c_int, *mut tabpage_T> =
    std::collections::BTreeMap::new();
pub const ui_ext_names: [&std::ffi::CStr; 10] = [
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
pub static mut last_cursormoved_win: *mut win_T = ptr::null_mut();
pub static mut last_cursormoved: pos_T = pos_T {
    lnum: 0,
    col: 0,
    coladd: 0,
};
pub static mut autocmd_busy: bool = false;
pub static mut autocmd_no_enter: c_int = 0;
pub static mut autocmd_no_leave: c_int = 0;
pub static mut au_new_curbuf: bufref_T = bufref_T {
    br_buf: ptr::null_mut(),
    br_fnum: 0,
    br_buf_free_count: 0,
};
pub static mut au_pending_free_buf: Vec<Box<buf_T>> = Vec::new();
pub static mut au_pending_free_win: Vec<Box<win_T>> = Vec::new();
pub static mut autocmd_fname: Option<std::ffi::CString> = None;
pub static mut autocmd_fname_full: bool = false;
pub static mut autocmd_bufnr: c_int = 0;
pub static mut autocmd_match: Option<std::ffi::CString> = None;
pub static mut did_cursorhold: bool = true;
pub static mut deferred_events: *mut MultiQueue = ptr::null_mut();
pub const msg_loclist: &std::ffi::CStr = c"[Location List]";
pub const msg_qflist: &std::ffi::CStr = c"[Quickfix List]";
pub static mut channels: std::collections::BTreeMap<u64, *mut Channel> =
    std::collections::BTreeMap::new();
pub static mut on_print: Callback = Callback::NONE;
pub static mut ctx_win_vec: Vec<CtxWin> = Vec::new();
pub const virt_text_pos_str: [&std::ffi::CStr; 6] = [
    c"eol",
    c"eol_right_align",
    c"inline",
    c"overlay",
    c"right_align",
    c"win_col",
];
pub const hl_mode_str: [&std::ffi::CStr; 4] = [
    c"",
    c"replace",
    c"combine",
    c"blend",
];
pub static mut decor_state: DecorState = DecorState::new();
pub static mut decor_items: Vec<DecorSignHighlight> = Vec::new();
pub static mut diff_context: c_int = 6;
pub static mut diff_foldcolumn: c_int = 2;
pub static mut diff_need_scrollbind: bool = false;
pub static mut need_diff_redraw: bool = false;
pub static mut win_extmark_arr: Vec<WinExtmark> = Vec::new();
pub static mut updating_screen: bool = false;
pub static mut redraw_not_allowed: bool = false;
pub static mut screen_search_hl: match_T = match_T {
    rm: crate::regexp::regmmatch_T {
        regprog: ptr::null_mut(),
        startpos: [crate::cursor::lpos_T { lnum: 0, col: 0 }; crate::regexp::NSUBEXP],
        endpos: [crate::cursor::lpos_T { lnum: 0, col: 0 }; crate::regexp::NSUBEXP],
        rmm_matchcol: 0,
        rmm_ic: 0,
        rmm_maxcol: 0,
    },
    buf: ptr::null_mut(),
    lnum: 0,
    attr: 0,
    attr_cur: 0,
    first_lnum: 0,
    startcol: 0,
    endcol: 0,
    is_addpos: false,
    has_cursor: false,
    tm: 0,
};
pub static mut search_hl_has_cursor_lnum: linenr_T = 0;
pub static mut EVALARG_EVALUATE: evalarg_T = evalarg_T { eval_flags: 1 };
pub static mut curbuf_splice_pending: c_int = 0;
pub static mut disable_fold_update: c_int = 0;
pub static mut default_grid: ScreenGrid = SCREEN_GRID_INIT;
pub static mut default_gridview: GridView = GridView {
    target: &raw mut default_grid,
    row_offset: 0,
    col_offset: 0,
};
pub static mut resizing_screen: bool = false;
pub static mut linebuf_char: Vec<schar_T> = Vec::new();
pub static mut linebuf_attr: Vec<sattr_T> = Vec::new();
pub static mut linebuf_vcol: Vec<colnr_T> = Vec::new();
pub const hlf_names: [Option<&std::ffi::CStr>; HLF_COUNT] = [
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

#[inline]
pub const fn hlf_name_ptr(idx: usize) -> *const c_char {
    match hlf_names[idx] {
        Some(s) => s.as_ptr(),
        None => ptr::null(),
    }
}
pub static mut highlight_attr: [c_int; HLF_COUNT] = [0; HLF_COUNT];
pub static mut highlight_attr_last: [c_int; HLF_COUNT] = [0; HLF_COUNT];
pub static mut highlight_user: [c_int; 9] = [0; 9];
pub static mut highlight_stlnc: [c_int; 9] = [0; 9];
pub static mut cterm_normal_fg_color: c_int = 0;
pub static mut cterm_normal_bg_color: c_int = 0;
pub static mut normal_fg: RgbValue = -1;
pub static mut normal_bg: RgbValue = -1;
pub static mut normal_sp: RgbValue = -1;
pub static mut ns_hl_global: NS = 0;
pub static mut ns_hl_win: NS = -1;
pub static mut ns_hl_fast: NS = -1;
pub static mut ns_hl_active: NS = 0;
pub static mut hl_attr_active: Option<std::rc::Rc<crate::highlight::NSHlAttr>> = None;
#[unsafe(no_mangle)]
pub static mut test_disable_char_avail: bool = false;
pub static mut nlua_global_refs: *mut nlua_ref_state_t = ptr::null_mut();
pub static mut nlua_disable_preload: bool = false;
pub static mut tslua_query_parse_count: u64 = 0;
pub static mut repeat_luamap: c_int = 0;
pub static mut namedfm: [xfmark_T; 36] = [const { xfmark_T::ZERO }; 36];
pub static mut arena_alloc_count: usize = 0;
pub static mut msg_ext_skip_flush: bool = false;
pub static mut msg_ext_overwrite: bool = false;
pub static mut msg_ext_skip_verbose: bool = false;
pub static mut msg_ext_fast: bool = true;
pub static mut msg_grid: ScreenGrid = SCREEN_GRID_INIT;
pub static mut msg_grid_pos: c_int = 0;
pub static mut msg_grid_adj: GridView = GridView {
    target: ptr::null_mut(),
    row_offset: 0,
    col_offset: 0,
};
pub static mut msg_scrolled_at_flush: c_int = 0;
pub static mut msg_grid_scroll_discount: c_int = 0;
pub static mut msg_listdo_overwrite: c_int = 0;
pub static mut ch_before_blocking_events: *mut MultiQueue = ptr::null_mut();
pub static mut showcmd_buf: [c_char; 51] = [0; 51];
pub static mut nvim_testing: bool = false;
pub static mut pum_grid: ScreenGrid = SCREEN_GRID_INIT;
pub static mut pum_want: PumWant = PumWant {
    active: false,
    item: 0,
    insert: false,
    finish: false,
};
pub static mut last_insert_ga: Vec<c_char> = Vec::new();
pub static mut add_last_insert: c_int = 0;
pub static mut tab_page_click_defs: Vec<StlClickDefinition> = Vec::new();
pub static mut ui_event_ns_id: u32 = 0;
pub static mut resize_events: *mut MultiQueue = ptr::null_mut();
pub static mut ui_refresh_cmdheight: bool = true;
pub static mut grid_line_buf_char: Vec<schar_T> = Vec::new();
pub static mut grid_line_buf_attr: Vec<sattr_T> = Vec::new();
pub static mut ui_client_channel_id: u64 = 0;
pub static mut ui_client_error_exit: c_int = -1;
pub static mut ui_client_exit_status: c_int = 0;
pub static mut ui_client_attached: bool = false;
pub static mut ui_client_forward_stdin: bool = false;
pub static mut tabpage_move_disallowed: c_int = 0;
pub const float_anchor_str: [&std::ffi::CStr; 4] = [
    c"NW",
    c"NE",
    c"SW",
    c"SE",
];

#[cfg(test)]
pub(crate) static TEST_GLOBAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn default_globals_match_c_initializers() {
        let _guard = TEST_GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            assert_eq!({ Rows }, 24);
            assert_eq!({ Columns }, 80);
            assert_eq!({ dollar_vcol }, -1);
            assert_eq!({ t_colors }, 256);
            assert_eq!({ starting }, 2);
            assert_eq!({ State }, 0x01);
            assert_eq!({ Visual.mode }, b'v' as c_int);
            assert_eq!({ Search.last_line }, 0x7fffffff);
            assert_eq!(
                CStr::from_ptr(no_lines_msg.as_ptr()).to_bytes(),
                b"--No lines in buffer--"
            );
        }
    }
}
