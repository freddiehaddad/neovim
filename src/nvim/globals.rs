//! Global variables (`src/nvim/globals.h`).
#![allow(non_upper_case_globals)]

use std::ffi::{c_char, c_int, c_long};
use std::mem::zeroed;
use std::ptr;

use crate::api::buffer::VisualState;
use crate::api::command::cmdmod_T;
use crate::api::private::converter::sctx_T;
use crate::api::tabpage::{frame_T, tabpage_T, win_T};
use crate::arglist::alist_T;
use crate::cursor::{colnr_T, linenr_T};
use crate::eval::caller_scope;
use crate::eval::deprecated::{buf_T, pos_T};
use crate::ex_eval::{except_T, msglist_T};
use crate::ex_getln::{disptick_T, hlf_T};
use crate::extmark::TriState;
use crate::fileio::FILE;
use crate::garray::garray_T;
use crate::mapping::typebuf_T;
use crate::r#match::SearchState;
use crate::menu::vimmenu_T;
use crate::os::fs::NvimStats;
use crate::regexp::reg_extmatch_T;
use crate::textformat::InsState;

const IOSIZE: usize = 1024 + 1;
const MAXPATHL: usize = 4096;
const MSG_BUF_LEN: usize = 480;
const MODE_MAX_LENGTH: usize = 4;
const DFLT_ROWS: c_int = 24;
const DFLT_COLS: c_int = 80;
const NO_SCREEN: c_int = 2;
const PROF_NONE: c_int = 0;
const SEA_NONE: c_int = 0;
const MODE_NORMAL: c_int = 0x01;
const MAXLNUM: linenr_T = 0x7fffffff;

const fn bytes_to_chars<const N: usize>(b: &[u8; N]) -> [c_char; N] {
    let mut out = [0 as c_char; N];
    let mut i = 0;
    while i < N {
        out[i] = b[i] as c_char;
        i += 1;
    }
    out
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut g_stats: NvimStats = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut Rows: c_int = DFLT_ROWS;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut Columns: c_int = DFLT_COLS;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut mod_mask: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut vgetc_mod_mask: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut vgetc_char: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmdline_row: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut redraw_cmdline: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut redraw_mode: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut clear_cmdline: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut mode_displayed: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmdline_star: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut redrawing_cmdline: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmdline_was_last_drawn: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut exec_from_reg: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut dollar_vcol: colnr_T = -1;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut edit_submode: *const c_char = ptr::null();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut edit_submode_pre: *const c_char = ptr::null();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut edit_submode_extra: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut edit_submode_highl: hlf_T = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmdmsg_rl: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_col: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_row: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_scrolled: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_scrolled_ign: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_did_scroll: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut keep_msg: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut keep_msg_hl_id: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut need_fileinfo: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_scroll: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_didout: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_didany: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_nowait: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut emsg_off: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut info_message: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_hist_off: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut need_clr_eos: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut emsg_skip: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut emsg_severe: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut emsg_assert_fails_msg: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut emsg_assert_fails_lnum: c_long = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut emsg_assert_fails_context: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_endif: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_emsg: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_beep: u64 = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_beep_silent: u64 = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_emsg_syntax: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut called_emsg: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ex_exitval: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut emsg_on_display: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut rc_did_emsg: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut no_wait_return: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut need_wait_return: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_wait_return: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut need_maketitle: bool = true;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut quit_more: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut vgetc_busy: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut didset_vim: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut didset_vimruntime: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut lines_left: c_int = -1;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_no_more: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ex_nesting_level: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut debug_break_level: c_int = -1;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut debug_did_msg: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut debug_tick: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut debug_backtrace_level: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut do_profiling: c_int = PROF_NONE;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut current_exception: *mut except_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_throw: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut need_rethrow: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut check_cstack: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut trylevel: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut force_abort: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_list: *mut *mut msglist_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut suppress_errthrow: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut caught_stack: *mut except_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut may_garbage_collect: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut want_garbage_collect: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut garbage_collect_at_exit: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut current_sctx: sctx_T = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut current_ui: u64 = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_source_packages: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut provider_caller_scope: caller_scope = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut provider_call_nesting: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut t_colors: c_int = 256;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut include_none: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut include_default: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut include_link: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut need_check_timestamps: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_check_timestamps: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut no_check_timestamps: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut mouse_grid: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut mouse_row: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut mouse_col: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut mouse_past_bottom: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut mouse_past_eol: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut mouse_dragging: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut root_menu: *mut vimmenu_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut sys_menu: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut firstwin: *mut win_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut lastwin: *mut win_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut prevwin: *mut win_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut curwin: *mut win_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut topframe: *mut frame_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut first_tabpage: *mut tabpage_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut curtab: *mut tabpage_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut lastused_tabpage: *mut tabpage_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut redraw_tabline: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut firstbuf: *mut buf_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut lastbuf: *mut buf_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut curbuf: *mut buf_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut global_alist: alist_T = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut max_alist_id: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut arg_had_last: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ru_col: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ru_wid: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut sc_col: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut starting: c_int = NO_SCREEN;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut exiting: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut v_dying: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut stdin_isatty: bool = true;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut stdout_isatty: bool = true;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut stderr_isatty: bool = true;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut stdin_fd: c_int = -1;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut full_screen: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut secure: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut textlock: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut allbuf_lock: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut sandbox: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut silent_mode: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut exmode_active: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut Visual: VisualState = VisualState {
    mode: b'v' as c_int,
    ..unsafe { zeroed() }
};
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut where_paste_started: pos_T = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_syncbind: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut old_indent: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut saved_cursor: pos_T = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut Ins: InsState = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut orig_line_count: linenr_T = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut vr_lines_changed: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut inhibit_delete_count: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut fenc_default: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut State: c_int = MODE_NORMAL;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut debug_mode: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut finish_op: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut opcount: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut motion_force: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmdpreview: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut reg_recording: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut reg_executing: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut pending_end_reg_executing: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut reg_recorded: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut no_mapping: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut no_zero_mapping: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut allow_keys: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut no_u_sync: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut u_sync_once: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut force_restart_edit: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut restart_edit: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ins_at_eol: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut no_abbr: bool = true;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut mapped_ctrl_c: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ctrl_c_interrupts: bool = true;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmdmod: cmdmod_T = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_silent: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut emsg_silent: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut emsg_noredir: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmd_silent: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut in_assert_fails: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut swap_exists_action: c_int = SEA_NONE;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut swap_exists_did_quit: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut IObuff: [c_char; IOSIZE] = [0 as c_char; IOSIZE];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut NameBuff: [c_char; MAXPATHL] = [0 as c_char; MAXPATHL];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut msg_buf: [c_char; MSG_BUF_LEN] = [0 as c_char; MSG_BUF_LEN];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut os_buf: [c_char; MAXPATHL] = [0 as c_char; MAXPATHL];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut RedrawingDisabled: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut readonlymode: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut recoverymode: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut typebuf: typebuf_T = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut typebuf_was_empty: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ex_normal_busy: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut expr_map_lock: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ignore_script: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut KeyTyped: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut KeyStuffed: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut maptick: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut must_redraw: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut skip_redraw: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut do_redraw: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut must_redraw_pum: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut need_highlight_changed: bool = true;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut scriptout: *mut FILE = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut got_int: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut bangredo: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut reg_do_extmatch: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut re_extmatch_in: *mut reg_extmatch_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut re_extmatch_out: *mut reg_extmatch_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_outofmem_msg: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_swapwrite_msg: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut global_busy: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut listcmd_busy: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut need_start_insertmode: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut last_mode: [c_char; MODE_MAX_LENGTH] = [b'n' as c_char, 0, 0, 0];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut last_cmdline: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut repeat_cmdline: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut new_last_cmdline: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut postponed_split: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut postponed_split_flags: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut postponed_split_tab: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut g_do_tagpreview: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut g_tag_at_cursor: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut replace_offset: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut escape_chars: *const c_char = c" \t\\\"|".as_ptr();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut keep_help_flag: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut redir_off: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut redir_fd: *mut FILE = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut redir_reg: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut redir_vname: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut capture_ga: *mut garray_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut langmap_mapchar: [u8; 256] = [0; 256];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut save_p_ls: c_int = -1;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut save_p_wmh: c_int = -1;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut wild_menu_showing: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut globaldir: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut last_chdir_reason: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut km_stopsel: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut km_startsel: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmdwin_type: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmdwin_buf: *mut buf_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmdline_win: *mut win_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static no_lines_msg: [c_char; 23] = bytes_to_chars(b"--No lines in buffer--\0");
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut sub_nsubs: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut sub_nlines: linenr_T = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut wim_flags: [u8; 4] = [0; 4];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut stl_syntax: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut typebuf_was_filled: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut virtual_op: TriState = -1;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut display_tick: disptick_T = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut spell_redraw_lnum: linenr_T = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut time_fd: *mut FILE = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut vim_ignored: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut embedded_mode: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut headless_mode: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut windowsVersion: [c_char; 20] = [0 as c_char; 20];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut skip_win_fix_scroll: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut skip_update_topline: bool = false;

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn default_globals_match_c_initializers() {
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
