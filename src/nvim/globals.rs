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
use crate::eval::typval::Callback;
use crate::event::multiqueue::MultiQueue;
use crate::ex_eval::{Except, Msglist};
use crate::ex_getln::{Disptick, Hlf};
use crate::extmark::TriState;
use crate::grid::{GridTarget, GridView, Sattr, Schar, ScreenGrid};
use crate::lua::converter::NluaRefState;
use crate::mapping::Typebuf;
use crate::r#match::{SearchState, SynMatch};
use crate::mouse::StlClickDefinition;
use crate::normal::VisualState;
use crate::os::fs::NvimStats;
use crate::popupmenu::PumWant;
use crate::runtime::Sctx;
use crate::state::{MODE_MAX_LENGTH, MODE_NORMAL};
use crate::textformat::InsState;
use crate::window::{Frame, TabPageInfo, WinInfoInfo};

pub(crate) const IOSIZE: usize = 1024 + 1;
pub(crate) const DFLT_ROWS: i32 = 24;
pub(crate) const DFLT_COLS: i32 = 80;
pub(crate) const PROF_NONE: i32 = 0;

pub struct WorkspaceState {
    curwin_rc: CloneCell<WinPtr>,
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
    pub curbuf_splice_pending: std::cell::Cell<i32>,
    buffer_handles: std::cell::RefCell<crate::memory::FxHashMap<i32, BufPtr>>,
    window_handles: std::cell::RefCell<crate::memory::FxHashMap<i32, WinPtr>>,
    tabpage_handles: std::cell::RefCell<crate::memory::FxHashMap<i32, TabpagePtr>>,
    pub postponed_split: std::cell::Cell<i32>,
    pub postponed_split_flags: std::cell::Cell<i32>,
    pub postponed_split_tab: std::cell::Cell<i32>,
    pub tabpage_move_disallowed: std::cell::Cell<i32>,
    pub skip_update_topline: std::cell::Cell<bool>,
    pub disable_fold_update: std::cell::Cell<i32>,
    pub diff_context: std::cell::Cell<i32>,
    pub diff_need_scrollbind: std::cell::Cell<bool>,
    pub need_diff_redraw: std::cell::Cell<bool>,
    pub did_syncbind: std::cell::Cell<bool>,
    pub globaldir: CloneCell<Option<std::ffi::CString>>,
    pub last_chdir_reason: std::cell::Cell<Option<&'static std::ffi::CStr>>,
    pub swap_exists_action: std::cell::Cell<i32>,
    pub swap_exists_did_quit: std::cell::Cell<bool>,
    pub did_swapwrite_msg: std::cell::Cell<bool>,
    pub need_check_timestamps: std::cell::Cell<bool>,
    pub did_check_timestamps: std::cell::Cell<bool>,
    pub no_check_timestamps: std::cell::Cell<i32>,
    ctx_win_vec: std::cell::RefCell<Vec<CtxWin>>,
}

impl WorkspaceState {
    pub const fn new() -> Self {
        Self {
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
            curbuf_splice_pending: std::cell::Cell::new(0),
            buffer_handles: std::cell::RefCell::new(crate::memory::fx_hashmap()),
            window_handles: std::cell::RefCell::new(crate::memory::fx_hashmap()),
            tabpage_handles: std::cell::RefCell::new(crate::memory::fx_hashmap()),
            postponed_split: std::cell::Cell::new(0),
            postponed_split_flags: std::cell::Cell::new(0),
            postponed_split_tab: std::cell::Cell::new(0),
            tabpage_move_disallowed: std::cell::Cell::new(0),
            skip_update_topline: std::cell::Cell::new(false),
            disable_fold_update: std::cell::Cell::new(0),
            diff_context: std::cell::Cell::new(6),
            diff_need_scrollbind: std::cell::Cell::new(false),
            need_diff_redraw: std::cell::Cell::new(false),
            did_syncbind: std::cell::Cell::new(false),
            globaldir: CloneCell::new(None),
            last_chdir_reason: std::cell::Cell::new(None),
            swap_exists_action: std::cell::Cell::new(SEA_NONE),
            swap_exists_did_quit: std::cell::Cell::new(false),
            did_swapwrite_msg: std::cell::Cell::new(false),
            need_check_timestamps: std::cell::Cell::new(false),
            did_check_timestamps: std::cell::Cell::new(false),
            no_check_timestamps: std::cell::Cell::new(0),
            ctx_win_vec: std::cell::RefCell::new(Vec::new()),
        }
    }

    #[inline(always)]
    pub fn curwin(&self) -> WinPtr {
        self.curwin_rc.get()
    }

    #[inline(always)]
    pub fn set_curwin(&self, wp: impl Into<WinPtr>) {
        let wp = wp.into();
        CURWIN_FFI.store(wp.as_ptr(), std::sync::atomic::Ordering::Relaxed);
        self.curwin_rc.set(wp);
    }

    #[inline]
    pub fn find_buffer(&self, handle: i32) -> Option<BufPtr> {
        self.buffer_handles
            .borrow()
            .get(&handle)
            .cloned()
            .filter(|p| !p.is_null())
    }

    #[inline]
    pub fn register_buffer(&self, handle: i32, buf: BufPtr) {
        let prev = self.buffer_handles.borrow_mut().insert(handle, buf);
        drop(prev);
    }

    #[inline]
    pub fn unregister_buffer(&self, handle: i32) {
        let prev = self.buffer_handles.borrow_mut().remove(&handle);
        drop(prev);
    }

    #[inline]
    pub fn find_window(&self, handle: i32) -> Option<WinPtr> {
        self.window_handles
            .borrow()
            .get(&handle)
            .cloned()
            .filter(|p| !p.is_null())
    }

    #[inline]
    pub fn register_window(&self, handle: i32, win: WinPtr) {
        let prev = self.window_handles.borrow_mut().insert(handle, win);
        drop(prev);
    }

    #[inline]
    pub fn unregister_window(&self, handle: i32) {
        let prev = self.window_handles.borrow_mut().remove(&handle);
        drop(prev);
    }

    #[inline]
    pub fn find_tabpage(&self, handle: i32) -> Option<TabpagePtr> {
        self.tabpage_handles
            .borrow()
            .get(&handle)
            .cloned()
            .filter(|p| !p.is_null())
    }

    #[inline]
    pub fn register_tabpage(&self, handle: i32, tp: TabpagePtr) {
        let prev = self.tabpage_handles.borrow_mut().insert(handle, tp);
        drop(prev);
    }

    #[inline]
    pub fn unregister_tabpage(&self, handle: i32) {
        let prev = self.tabpage_handles.borrow_mut().remove(&handle);
        drop(prev);
    }

    pub fn clear_handles(&self) {
        let bufs = std::mem::replace(
            &mut *self.buffer_handles.borrow_mut(),
            crate::memory::fx_hashmap(),
        );
        let wins = std::mem::replace(
            &mut *self.window_handles.borrow_mut(),
            crate::memory::fx_hashmap(),
        );
        let tabs = std::mem::replace(
            &mut *self.tabpage_handles.borrow_mut(),
            crate::memory::fx_hashmap(),
        );
        drop((bufs, wins, tabs));
    }

    pub fn is_ctx_win(&self, win: &WinInfoInfo) -> bool {
        self.ctx_win_vec
            .borrow()
            .iter()
            .any(|item| item.cw_used && item.cw_win == std::ptr::from_ref(win))
    }

    pub fn acquire_ctx_win_slot(&self) -> (usize, bool) {
        let pos = self
            .ctx_win_vec
            .borrow()
            .iter()
            .position(|item| !item.cw_used);
        let idx = pos.unwrap_or_else(|| {
            let mut vec = self.ctx_win_vec.borrow_mut();
            let idx = vec.len();
            vec.push(CtxWin {
                cw_win: WinPtr::NULL,
                cw_used: false,
            });
            idx
        });
        let is_null = self.ctx_win_vec.borrow()[idx].cw_win.is_null();
        (idx, is_null)
    }

    #[inline]
    pub fn set_ctx_win_slot(&self, idx: usize, win: WinPtr) {
        let prev = std::mem::replace(&mut self.ctx_win_vec.borrow_mut()[idx].cw_win, win);
        drop(prev);
    }

    #[inline]
    pub fn activate_ctx_win(&self, idx: usize) -> WinPtr {
        let mut vec = self.ctx_win_vec.borrow_mut();
        vec[idx].cw_used = true;
        vec[idx].cw_win.clone()
    }

    #[inline]
    pub fn ctx_win_at(&self, idx: usize) -> WinPtr {
        self.ctx_win_vec.borrow()[idx].cw_win.clone()
    }

    #[inline]
    pub fn deactivate_ctx_win(&self, idx: usize) {
        self.ctx_win_vec.borrow_mut()[idx].cw_used = false;
    }

    pub fn clear_ctx_win_matching(&self, wp: &WinPtr) {
        let mut cleared: Vec<WinPtr> = Vec::new();
        for item in self.ctx_win_vec.borrow_mut().iter_mut() {
            if item.cw_win == wp {
                cleared.push(std::mem::replace(&mut item.cw_win, WinPtr::NULL));
            }
        }
        drop(cleared);
    }

    #[inline]
    pub fn ctx_win_len(&self) -> usize {
        self.ctx_win_vec.borrow().len()
    }

    pub fn clear_ctx_wins(&self) {
        let prev = std::mem::take(&mut *self.ctx_win_vec.borrow_mut());
        drop(prev);
    }

    pub fn active_ctx_wins(&self) -> Vec<WinPtr> {
        self.ctx_win_vec
            .borrow()
            .iter()
            .filter(|cw| !cw.cw_win.is_null())
            .map(|cw| cw.cw_win.clone())
            .collect()
    }
}

pub struct MessageGlobalState {
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
    pub did_emsg: std::cell::Cell<i32>,
    pub did_beep: std::cell::Cell<u64>,
    pub did_beep_silent: std::cell::Cell<u64>,
    pub called_emsg: std::cell::Cell<i32>,
    pub emsg_on_display: std::cell::Cell<bool>,
    pub rc_did_emsg: std::cell::Cell<bool>,
    pub no_wait_return: std::cell::Cell<i32>,
    pub need_wait_return: std::cell::Cell<bool>,
    pub did_wait_return: std::cell::Cell<bool>,
    pub quit_more: std::cell::Cell<bool>,
    pub lines_left: std::cell::Cell<i32>,
    pub msg_no_more: std::cell::Cell<bool>,
    pub msg_silent: std::cell::Cell<i32>,
    pub emsg_silent: std::cell::Cell<i32>,
    pub emsg_noredir: std::cell::Cell<bool>,
    pub cmd_silent: std::cell::Cell<bool>,
    pub in_assert_fails: std::cell::Cell<bool>,
    pub redir_off: std::cell::Cell<bool>,
    redir_fd: std::cell::RefCell<Option<crate::os::fs::CFile>>,
    pub redir_reg: std::cell::Cell<i32>,
    pub redir_vname: std::cell::Cell<bool>,
    capture_ga: std::cell::RefCell<Option<Vec<u8>>>,
    pub on_print: Callback,
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
}

impl MessageGlobalState {
    pub const fn new() -> Self {
        Self {
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
            did_emsg: std::cell::Cell::new(0),
            did_beep: std::cell::Cell::new(0),
            did_beep_silent: std::cell::Cell::new(0),
            called_emsg: std::cell::Cell::new(0),
            emsg_on_display: std::cell::Cell::new(false),
            rc_did_emsg: std::cell::Cell::new(false),
            no_wait_return: std::cell::Cell::new(0),
            need_wait_return: std::cell::Cell::new(false),
            did_wait_return: std::cell::Cell::new(false),
            quit_more: std::cell::Cell::new(false),
            lines_left: std::cell::Cell::new(-1),
            msg_no_more: std::cell::Cell::new(false),
            msg_silent: std::cell::Cell::new(0),
            emsg_silent: std::cell::Cell::new(0),
            emsg_noredir: std::cell::Cell::new(false),
            cmd_silent: std::cell::Cell::new(false),
            in_assert_fails: std::cell::Cell::new(false),
            redir_off: std::cell::Cell::new(false),
            redir_fd: std::cell::RefCell::new(None),
            redir_reg: std::cell::Cell::new(0),
            redir_vname: std::cell::Cell::new(false),
            capture_ga: std::cell::RefCell::new(None),
            on_print: Callback::NONE,
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
        }
    }

    #[inline(always)]
    pub fn has_redir_fd(&self) -> bool {
        self.redir_fd.borrow().is_some()
    }

    #[inline]
    pub fn set_redir_fd(&self, fd: Option<crate::os::fs::CFile>) {
        let prev = std::mem::replace(&mut *self.redir_fd.borrow_mut(), fd);
        drop(prev);
    }

    #[inline]
    pub fn redir_puts(&self, s: &std::ffi::CStr) {
        if let Some(ref rf) = *self.redir_fd.borrow() {
            rf.puts(s);
        }
    }

    #[inline]
    pub fn redir_putc(&self, c: i32) {
        if let Some(ref rf) = *self.redir_fd.borrow() {
            rf.putc(c);
        }
    }

    #[inline(always)]
    pub fn is_capturing(&self) -> bool {
        self.capture_ga.borrow().is_some()
    }

    #[inline]
    pub fn take_capture(&self) -> Option<Vec<u8>> {
        self.capture_ga.take()
    }

    #[inline]
    pub fn set_capture(&self, buf: Option<Vec<u8>>) {
        let prev = std::mem::replace(&mut *self.capture_ga.borrow_mut(), buf);
        drop(prev);
    }

    #[inline]
    pub fn replace_capture(&self, buf: Option<Vec<u8>>) -> Option<Vec<u8>> {
        self.capture_ga.replace(buf)
    }

    #[inline]
    pub fn append_capture_byte(&self, b: u8) {
        if let Some(ref mut cap) = *self.capture_ga.borrow_mut() {
            cap.push(b);
        }
    }

    #[inline]
    pub fn append_capture_slice(&self, slice: &[u8]) {
        if !slice.is_empty()
            && let Some(ref mut cap) = *self.capture_ga.borrow_mut()
        {
            cap.extend_from_slice(slice);
        }
    }
}

pub struct ExceptionGlobalState {
    pub g_stats: NvimStats,
    pub did_endif: std::cell::Cell<bool>,
    pub ex_exitval: std::cell::Cell<i32>,
    pub ex_nesting_level: std::cell::Cell<i32>,
    pub debug_break_level: std::cell::Cell<i32>,
    pub debug_did_msg: std::cell::Cell<bool>,
    pub debug_tick: std::cell::Cell<i32>,
    pub debug_backtrace_level: std::cell::Cell<i32>,
    pub do_profiling: std::cell::Cell<i32>,
    pub debug_mode: std::cell::Cell<bool>,
    pub current_exception: CloneCell<Option<std::rc::Rc<Except>>>,
    pub did_throw: std::cell::Cell<bool>,
    pub need_rethrow: std::cell::Cell<bool>,
    pub trylevel: std::cell::Cell<i32>,
    pub force_abort: std::cell::Cell<bool>,
    msg_list: std::cell::RefCell<Option<Vec<Msglist>>>,
    pub suppress_errthrow: std::cell::Cell<bool>,
    caught_stack: std::cell::RefCell<Vec<std::rc::Rc<Except>>>,
    pub may_garbage_collect: std::cell::Cell<bool>,
    pub want_garbage_collect: std::cell::Cell<bool>,
    pub garbage_collect_at_exit: std::cell::Cell<bool>,
    pub current_sctx: std::cell::Cell<Sctx>,
    pub nlua_disable_preload: std::cell::Cell<bool>,
    pub tslua_query_parse_count: std::cell::Cell<u64>,
    pub nvim_testing: std::cell::Cell<bool>,
    pub fenc_default: CloneCell<Option<std::ffi::CString>>,
    pub exiting: std::cell::Cell<bool>,
    pub v_dying: std::cell::Cell<i32>,
    pub stdin_isatty: std::cell::Cell<bool>,
    pub stdout_isatty: std::cell::Cell<bool>,
    pub stderr_isatty: std::cell::Cell<bool>,
    pub stdin_fd: std::cell::Cell<i32>,
    pub embedded_mode: std::cell::Cell<bool>,
    pub headless_mode: std::cell::Cell<bool>,
    pub readonlymode: std::cell::Cell<bool>,
    pub recoverymode: std::cell::Cell<bool>,
    pub windows_version: std::cell::Cell<[u8; 20]>,
    pub didset_vim: std::cell::Cell<bool>,
    pub didset_vimruntime: std::cell::Cell<bool>,
    pub vim_ignored: std::cell::Cell<i32>,
}

impl ExceptionGlobalState {
    pub const fn new() -> Self {
        Self {
            g_stats: NvimStats {
                fsync: std::cell::Cell::new(0),
                redraw: std::cell::Cell::new(0),
                log_skip: std::cell::Cell::new(0),
            },
            did_endif: std::cell::Cell::new(false),
            ex_exitval: std::cell::Cell::new(0),
            ex_nesting_level: std::cell::Cell::new(0),
            debug_break_level: std::cell::Cell::new(-1),
            debug_did_msg: std::cell::Cell::new(false),
            debug_tick: std::cell::Cell::new(0),
            debug_backtrace_level: std::cell::Cell::new(0),
            do_profiling: std::cell::Cell::new(PROF_NONE),
            debug_mode: std::cell::Cell::new(false),
            current_exception: CloneCell::new(None),
            did_throw: std::cell::Cell::new(false),
            need_rethrow: std::cell::Cell::new(false),
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
            nlua_disable_preload: std::cell::Cell::new(false),
            tslua_query_parse_count: std::cell::Cell::new(0),
            nvim_testing: std::cell::Cell::new(false),
            fenc_default: CloneCell::new(None),
            exiting: std::cell::Cell::new(false),
            v_dying: std::cell::Cell::new(0),
            stdin_isatty: std::cell::Cell::new(true),
            stdout_isatty: std::cell::Cell::new(true),
            stderr_isatty: std::cell::Cell::new(true),
            stdin_fd: std::cell::Cell::new(-1),
            embedded_mode: std::cell::Cell::new(false),
            headless_mode: std::cell::Cell::new(false),
            readonlymode: std::cell::Cell::new(false),
            recoverymode: std::cell::Cell::new(false),
            windows_version: std::cell::Cell::new([0u8; 20]),
            didset_vim: std::cell::Cell::new(false),
            didset_vimruntime: std::cell::Cell::new(false),
            vim_ignored: std::cell::Cell::new(0),
        }
    }

    #[inline]
    pub fn push_caught(&self, exc: std::rc::Rc<Except>) {
        self.caught_stack.borrow_mut().push(exc);
    }

    #[inline]
    pub fn pop_caught(&self) -> Option<std::rc::Rc<Except>> {
        self.caught_stack.borrow_mut().pop()
    }

    #[inline]
    pub fn peek_caught(&self) -> Option<std::rc::Rc<Except>> {
        self.caught_stack.borrow().last().cloned()
    }

    #[inline]
    pub fn take_caught_stack(&self) -> Vec<std::rc::Rc<Except>> {
        self.caught_stack.take()
    }

    #[inline]
    pub fn set_caught_stack(&self, stack: Vec<std::rc::Rc<Except>>) {
        let prev = std::mem::replace(&mut *self.caught_stack.borrow_mut(), stack);
        drop(prev);
    }

    #[inline]
    pub fn has_msg_list(&self) -> bool {
        self.msg_list.borrow().is_some()
    }

    #[inline]
    pub fn has_nonempty_msg_list(&self) -> bool {
        self.msg_list
            .borrow()
            .as_ref()
            .is_some_and(|msgs| !msgs.is_empty())
    }

    #[inline]
    pub fn take_msg_list(&self) -> Option<Vec<Msglist>> {
        self.msg_list.take()
    }

    #[inline]
    pub fn take_pending_messages(&self) -> Option<Vec<Msglist>> {
        let mut guard = self.msg_list.borrow_mut();
        let list = guard.as_mut()?;
        if list.is_empty() {
            return None;
        }
        Some(std::mem::take(list))
    }

    #[inline]
    pub fn replace_msg_list(&self, list: Option<Vec<Msglist>>) -> Option<Vec<Msglist>> {
        self.msg_list.replace(list)
    }

    #[inline]
    pub fn set_msg_list(&self, list: Option<Vec<Msglist>>) {
        let prev = std::mem::replace(&mut *self.msg_list.borrow_mut(), list);
        drop(prev);
    }

    #[inline]
    pub fn clear_msg_list(&self) {
        if let Some(ref mut list) = *self.msg_list.borrow_mut() {
            list.clear();
        }
    }

    pub fn with_msg_list_mut<R>(&self, f: impl FnOnce(Option<&mut Vec<Msglist>>) -> R) -> R {
        let mut guard = self.msg_list.borrow_mut();
        f(guard.as_mut())
    }
}

pub struct ScreenLineBuf {
    pub chars: Vec<Schar>,
    pub attrs: Vec<Sattr>,
    pub vcols: Vec<Colnr>,
}

impl ScreenLineBuf {
    pub const fn new() -> Self {
        Self {
            chars: Vec::new(),
            attrs: Vec::new(),
            vcols: Vec::new(),
        }
    }
}

pub struct ScreenGlobalState {
    pub rows_global: std::cell::Cell<i32>,
    pub columns_global: std::cell::Cell<i32>,
    pub t_colors: std::cell::Cell<i32>,
    pub include_none: std::cell::Cell<i32>,
    pub include_default: std::cell::Cell<i32>,
    pub include_link: std::cell::Cell<i32>,
    pub full_screen: std::cell::Cell<bool>,
    pub need_maketitle: std::cell::Cell<bool>,
    pub ru_col: std::cell::Cell<i32>,
    pub ru_wid: std::cell::Cell<i32>,
    pub sc_col: std::cell::Cell<i32>,
    pub stl_syntax: std::cell::Cell<i32>,
    pub spell_redraw_lnum: std::cell::Cell<Linenr>,
    pub redrawing_disabled: std::cell::Cell<i32>,
    pub must_redraw: std::cell::Cell<i32>,
    pub skip_redraw: std::cell::Cell<bool>,
    pub do_redraw: std::cell::Cell<bool>,
    pub must_redraw_pum: std::cell::Cell<bool>,
    pub need_highlight_changed: std::cell::Cell<bool>,
    pub updating_screen: std::cell::Cell<bool>,
    pub redraw_not_allowed: std::cell::Cell<bool>,
    pub resizing_screen: std::cell::Cell<bool>,
    pub default_grid: ScreenGrid,
    pub default_gridview: GridView,
    linebuf: std::cell::RefCell<ScreenLineBuf>,
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
    pub ns_hl_fast: std::cell::Cell<NS>,
    pub ns_hl_active: std::cell::Cell<NS>,
    decor_state: std::cell::RefCell<DecorState>,
    win_extmark_arr: std::cell::RefCell<Vec<WinExtmark>>,
    pub pum_grid: ScreenGrid,
    pub pum_want: PumWant,
    tab_page_click_defs: std::cell::RefCell<Vec<StlClickDefinition>>,
    pub current_ui: std::cell::Cell<u64>,
    pub ui_event_ns_id: std::cell::Cell<u32>,
    pub resize_events: CloneCell<MultiQueue>,
    pub ui_refresh_cmdheight: std::cell::Cell<bool>,
    pub ui_client_channel_id: std::cell::Cell<u64>,
    pub ui_client_error_exit: std::cell::Cell<i32>,
    pub ui_client_exit_status: std::cell::Cell<i32>,
    pub ui_client_attached: std::cell::Cell<bool>,
    pub ui_client_forward_stdin: std::cell::Cell<bool>,
}

impl ScreenGlobalState {
    pub const fn new() -> Self {
        Self {
            rows_global: std::cell::Cell::new(DFLT_ROWS),
            columns_global: std::cell::Cell::new(DFLT_COLS),
            t_colors: std::cell::Cell::new(256),
            include_none: std::cell::Cell::new(0),
            include_default: std::cell::Cell::new(0),
            include_link: std::cell::Cell::new(0),
            full_screen: std::cell::Cell::new(false),
            need_maketitle: std::cell::Cell::new(true),
            ru_col: std::cell::Cell::new(0),
            ru_wid: std::cell::Cell::new(0),
            sc_col: std::cell::Cell::new(0),
            stl_syntax: std::cell::Cell::new(0),
            spell_redraw_lnum: std::cell::Cell::new(0),
            redrawing_disabled: std::cell::Cell::new(0),
            must_redraw: std::cell::Cell::new(0),
            skip_redraw: std::cell::Cell::new(false),
            do_redraw: std::cell::Cell::new(false),
            must_redraw_pum: std::cell::Cell::new(false),
            need_highlight_changed: std::cell::Cell::new(true),
            updating_screen: std::cell::Cell::new(false),
            redraw_not_allowed: std::cell::Cell::new(false),
            resizing_screen: std::cell::Cell::new(false),
            default_grid: ScreenGrid::with_target(GridTarget::Default),
            default_gridview: GridView {
                target: std::cell::Cell::new(GridTarget::Default),
                row_offset: std::cell::Cell::new(0),
                col_offset: std::cell::Cell::new(0),
            },
            linebuf: std::cell::RefCell::new(ScreenLineBuf::new()),
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
            ns_hl_fast: std::cell::Cell::new(-1),
            ns_hl_active: std::cell::Cell::new(0),
            decor_state: std::cell::RefCell::new(DecorState::new()),
            win_extmark_arr: std::cell::RefCell::new(Vec::new()),
            pum_grid: ScreenGrid::with_target(GridTarget::Pum),
            pum_want: PumWant {
                active: std::cell::Cell::new(false),
                item: std::cell::Cell::new(0),
                insert: std::cell::Cell::new(false),
                finish: std::cell::Cell::new(false),
            },
            tab_page_click_defs: std::cell::RefCell::new(Vec::new()),
            current_ui: std::cell::Cell::new(0),
            ui_event_ns_id: std::cell::Cell::new(0),
            resize_events: CloneCell::new(MultiQueue::NULL),
            ui_refresh_cmdheight: std::cell::Cell::new(true),
            ui_client_channel_id: std::cell::Cell::new(0),
            ui_client_error_exit: std::cell::Cell::new(-1),
            ui_client_exit_status: std::cell::Cell::new(0),
            ui_client_attached: std::cell::Cell::new(false),
            ui_client_forward_stdin: std::cell::Cell::new(false),
        }
    }

    #[inline]
    pub fn with_linebuf<R>(&self, f: impl FnOnce(&ScreenLineBuf) -> R) -> R {
        f(&self.linebuf.borrow())
    }

    #[inline]
    pub fn with_linebuf_mut<R>(&self, f: impl FnOnce(&mut ScreenLineBuf) -> R) -> R {
        f(&mut self.linebuf.borrow_mut())
    }

    #[inline]
    pub fn with_decor_state<R>(&self, f: impl FnOnce(&DecorState) -> R) -> R {
        f(&self.decor_state.borrow())
    }

    #[inline]
    pub fn with_decor_state_mut<R>(&self, f: impl FnOnce(&mut DecorState) -> R) -> R {
        f(&mut self.decor_state.borrow_mut())
    }

    #[inline]
    pub fn replace_decor_state(&self, ds: DecorState) -> DecorState {
        self.decor_state.replace(ds)
    }

    #[inline]
    pub fn set_decor_state(&self, ds: DecorState) {
        let prev = std::mem::replace(&mut *self.decor_state.borrow_mut(), ds);
        drop(prev);
    }

    #[inline]
    pub fn clear_win_extmarks(&self) {
        self.win_extmark_arr.borrow_mut().clear();
    }

    #[inline]
    pub fn push_win_extmark(&self, m: WinExtmark) {
        self.win_extmark_arr.borrow_mut().push(m);
    }

    #[inline]
    pub fn win_extmarks_len(&self) -> usize {
        self.win_extmark_arr.borrow().len()
    }

    #[inline]
    pub fn win_extmark_at(&self, idx: usize) -> WinExtmark {
        self.win_extmark_arr.borrow()[idx]
    }

    #[inline]
    pub fn with_tab_page_click_defs<R>(&self, f: impl FnOnce(&[StlClickDefinition]) -> R) -> R {
        f(&self.tab_page_click_defs.borrow())
    }

    #[inline]
    pub fn with_tab_page_click_defs_mut<R>(
        &self,
        f: impl FnOnce(&mut Vec<StlClickDefinition>) -> R,
    ) -> R {
        f(&mut self.tab_page_click_defs.borrow_mut())
    }

    #[inline]
    pub fn set_tab_page_click_def(&self, idx: usize, def: StlClickDefinition) {
        let prev = std::mem::replace(&mut self.tab_page_click_defs.borrow_mut()[idx], def);
        drop(prev);
    }

    #[inline]
    pub fn clear_tab_page_click_defs(&self) {
        let prev = std::mem::take(&mut *self.tab_page_click_defs.borrow_mut());
        drop(prev);
    }
}

pub struct InputGlobalState {
    pub mod_mask: std::cell::Cell<i32>,
    pub vgetc_mod_mask: std::cell::Cell<i32>,
    pub vgetc_char: std::cell::Cell<i32>,
    pub vgetc_busy: std::cell::Cell<i32>,
    pub exec_from_reg: std::cell::Cell<bool>,
    pub reg_recording: std::cell::Cell<i32>,
    pub reg_executing: std::cell::Cell<i32>,
    pub pending_end_reg_executing: std::cell::Cell<bool>,
    pub reg_recorded: std::cell::Cell<i32>,
    pub no_mapping: std::cell::Cell<i32>,
    pub no_zero_mapping: std::cell::Cell<i32>,
    pub allow_keys: std::cell::Cell<i32>,
    pub mapped_ctrl_c: std::cell::Cell<i32>,
    pub ctrl_c_interrupts: std::cell::Cell<bool>,
    pub got_int: std::cell::Cell<bool>,
    pub typebuf: Typebuf,
    pub typebuf_was_filled: std::cell::Cell<bool>,
    pub ex_normal_busy: std::cell::Cell<i32>,
    pub expr_map_lock: std::cell::Cell<i32>,
    pub ignore_script: std::cell::Cell<bool>,
    pub key_typed: std::cell::Cell<bool>,
    pub key_stuffed: std::cell::Cell<i32>,
    pub maptick: std::cell::Cell<i32>,
    pub repeat_luamap: std::cell::Cell<i32>,
    scriptout: std::cell::RefCell<Option<crate::os::fs::CFile>>,
    pub langmap_mapchar: [std::cell::Cell<u8>; 256],
    pub ch_before_blocking_events: CloneCell<MultiQueue>,
    pub mouse_grid: std::cell::Cell<i32>,
    pub mouse_row: std::cell::Cell<i32>,
    pub mouse_col: std::cell::Cell<i32>,
    pub mouse_dragging: std::cell::Cell<i32>,
    pub secure: std::cell::Cell<i32>,
    pub textlock: std::cell::Cell<i32>,
    pub allbuf_lock: std::cell::Cell<i32>,
    pub sandbox: std::cell::Cell<i32>,
}

impl InputGlobalState {
    pub const fn new() -> Self {
        Self {
            mod_mask: std::cell::Cell::new(0),
            vgetc_mod_mask: std::cell::Cell::new(0),
            vgetc_char: std::cell::Cell::new(0),
            vgetc_busy: std::cell::Cell::new(0),
            exec_from_reg: std::cell::Cell::new(false),
            reg_recording: std::cell::Cell::new(0),
            reg_executing: std::cell::Cell::new(0),
            pending_end_reg_executing: std::cell::Cell::new(false),
            reg_recorded: std::cell::Cell::new(0),
            no_mapping: std::cell::Cell::new(0),
            no_zero_mapping: std::cell::Cell::new(0),
            allow_keys: std::cell::Cell::new(0),
            mapped_ctrl_c: std::cell::Cell::new(0),
            ctrl_c_interrupts: std::cell::Cell::new(true),
            got_int: std::cell::Cell::new(false),
            typebuf: Typebuf::ZERO,
            typebuf_was_filled: std::cell::Cell::new(false),
            ex_normal_busy: std::cell::Cell::new(0),
            expr_map_lock: std::cell::Cell::new(0),
            ignore_script: std::cell::Cell::new(false),
            key_typed: std::cell::Cell::new(false),
            key_stuffed: std::cell::Cell::new(0),
            maptick: std::cell::Cell::new(0),
            repeat_luamap: std::cell::Cell::new(0),
            scriptout: std::cell::RefCell::new(None),
            langmap_mapchar: [const { std::cell::Cell::new(0) }; 256],
            ch_before_blocking_events: CloneCell::new(MultiQueue::NULL),
            mouse_grid: std::cell::Cell::new(0),
            mouse_row: std::cell::Cell::new(0),
            mouse_col: std::cell::Cell::new(0),
            mouse_dragging: std::cell::Cell::new(0),
            secure: std::cell::Cell::new(0),
            textlock: std::cell::Cell::new(0),
            allbuf_lock: std::cell::Cell::new(0),
            sandbox: std::cell::Cell::new(0),
        }
    }

    #[inline(always)]
    pub fn has_scriptout(&self) -> bool {
        self.scriptout.borrow().is_some()
    }

    #[inline]
    pub fn set_scriptout(&self, f: Option<crate::os::fs::CFile>) {
        let prev = std::mem::replace(&mut *self.scriptout.borrow_mut(), f);
        drop(prev);
    }

    #[inline]
    pub fn take_scriptout(&self) -> Option<crate::os::fs::CFile> {
        self.scriptout.take()
    }

    #[inline]
    pub fn scriptout_putc(&self, c: i32) {
        if let Some(ref so) = *self.scriptout.borrow() {
            so.putc(c);
        }
    }
}

pub struct CmdlineGlobalState {
    pub cmdline_row: std::cell::Cell<i32>,
    pub redraw_cmdline: std::cell::Cell<bool>,
    pub redraw_mode: std::cell::Cell<bool>,
    pub clear_cmdline: std::cell::Cell<bool>,
    pub mode_displayed: std::cell::Cell<bool>,
    pub cmdline_star: std::cell::Cell<i32>,
    pub redrawing_cmdline: std::cell::Cell<bool>,
    pub cmdline_was_last_drawn: std::cell::Cell<bool>,
    pub silent_mode: std::cell::Cell<bool>,
    pub exmode_active: std::cell::Cell<bool>,
    pub cmdpreview: std::cell::Cell<bool>,
    cmdmod: std::cell::RefCell<Cmdmod>,
    pub bangredo: std::cell::Cell<bool>,
    pub global_busy: std::cell::Cell<i32>,
    pub listcmd_busy: std::cell::Cell<bool>,
    pub keep_help_flag: std::cell::Cell<bool>,
    pub last_cmdline: CloneCell<Option<std::ffi::CString>>,
    pub repeat_cmdline: CloneCell<Option<std::ffi::CString>>,
    pub new_last_cmdline: CloneCell<Option<std::ffi::CString>>,
    pub wild_menu_showing: std::cell::Cell<i32>,
    pub wim_flags: [std::cell::Cell<u8>; 4],
    pub cmdwin_type: std::cell::Cell<i32>,
    pub cmdwin_buf: CloneCell<BufPtr>,
    pub cmdline_win: CloneCell<WinPtr>,
}

impl CmdlineGlobalState {
    pub const fn new() -> Self {
        Self {
            cmdline_row: std::cell::Cell::new(0),
            redraw_cmdline: std::cell::Cell::new(false),
            redraw_mode: std::cell::Cell::new(false),
            clear_cmdline: std::cell::Cell::new(false),
            mode_displayed: std::cell::Cell::new(false),
            cmdline_star: std::cell::Cell::new(0),
            redrawing_cmdline: std::cell::Cell::new(false),
            cmdline_was_last_drawn: std::cell::Cell::new(false),
            silent_mode: std::cell::Cell::new(false),
            exmode_active: std::cell::Cell::new(false),
            cmdpreview: std::cell::Cell::new(false),
            cmdmod: std::cell::RefCell::new(Cmdmod::new()),
            bangredo: std::cell::Cell::new(false),
            global_busy: std::cell::Cell::new(0),
            listcmd_busy: std::cell::Cell::new(false),
            keep_help_flag: std::cell::Cell::new(false),
            last_cmdline: CloneCell::new(None),
            repeat_cmdline: CloneCell::new(None),
            new_last_cmdline: CloneCell::new(None),
            wild_menu_showing: std::cell::Cell::new(0),
            wim_flags: [const { std::cell::Cell::new(0) }; 4],
            cmdwin_type: std::cell::Cell::new(0),
            cmdwin_buf: CloneCell::new(BufPtr::NULL),
            cmdline_win: CloneCell::new(WinPtr::NULL),
        }
    }

    #[inline]
    pub fn cmod_flags(&self) -> i32 {
        self.cmdmod.borrow().cmod_flags
    }

    #[inline]
    pub fn set_cmod_flags(&self, flags: i32) {
        self.cmdmod.borrow_mut().cmod_flags = flags;
    }

    #[inline]
    pub fn add_cmod_flags(&self, flags: i32) {
        self.cmdmod.borrow_mut().cmod_flags |= flags;
    }

    #[inline]
    pub fn remove_cmod_flags(&self, flags: i32) {
        self.cmdmod.borrow_mut().cmod_flags &= !flags;
    }

    #[inline]
    pub fn cmod_split(&self) -> i32 {
        self.cmdmod.borrow().cmod_split
    }

    #[inline]
    pub fn set_cmod_split(&self, split: i32) {
        self.cmdmod.borrow_mut().cmod_split = split;
    }

    #[inline]
    pub fn add_cmod_split(&self, split: i32) {
        self.cmdmod.borrow_mut().cmod_split |= split;
    }

    #[inline]
    pub fn cmod_tab(&self) -> i32 {
        self.cmdmod.borrow().cmod_tab
    }

    #[inline]
    pub fn set_cmod_tab(&self, tab: i32) {
        self.cmdmod.borrow_mut().cmod_tab = tab;
    }

    #[inline]
    pub fn cmod_did_esilent(&self) -> i32 {
        self.cmdmod.borrow().cmod_did_esilent
    }

    #[inline]
    pub fn set_cmod_did_esilent(&self, v: i32) {
        self.cmdmod.borrow_mut().cmod_did_esilent = v;
    }

    #[inline]
    pub fn cmod_filter_force(&self) -> bool {
        self.cmdmod.borrow().cmod_filter_force
    }

    #[inline]
    pub fn take_cmod_filter_regmatch(&self) -> crate::regexp::Regmatch {
        std::mem::take(&mut self.cmdmod.borrow_mut().cmod_filter_regmatch)
    }

    #[inline]
    pub fn set_cmod_filter_regmatch(&self, rm: crate::regexp::Regmatch) {
        self.cmdmod.borrow_mut().cmod_filter_regmatch = rm;
    }

    #[inline]
    pub fn snapshot_cmdmod(&self) -> Cmdmod {
        self.cmdmod.borrow().clone()
    }

    #[inline]
    pub fn set_cmdmod(&self, cmod: Cmdmod) {
        let prev = std::mem::replace(&mut *self.cmdmod.borrow_mut(), cmod);
        drop(prev);
    }

    #[inline]
    pub fn cmod_has_filter(&self) -> bool {
        !self.cmdmod.borrow().cmod_filter_regmatch.regprog.is_null()
    }

    #[inline]
    pub fn take_cmdmod(&self) -> Cmdmod {
        self.cmdmod.take()
    }

    #[inline]
    pub fn replace_cmdmod(&self, cmod: Cmdmod) -> Cmdmod {
        std::mem::replace(&mut *self.cmdmod.borrow_mut(), cmod)
    }
}

pub struct AutocmdGlobalState {
    pub last_cursormoved_win: CloneCell<WinPtr>,
    pub last_cursormoved: std::cell::Cell<CursorPos>,
    pub did_cursorhold: std::cell::Cell<bool>,
    pub autocmd_busy: std::cell::Cell<bool>,
    pub autocmd_no_enter: std::cell::Cell<i32>,
    pub autocmd_no_leave: std::cell::Cell<i32>,
    pub au_new_curbuf: CloneCell<Bufref>,
    au_pending_free_buf: std::cell::RefCell<Vec<BufPtr>>,
    au_pending_free_win: std::cell::RefCell<Vec<WinPtr>>,
    pub autocmd_fname: CloneCell<Option<std::ffi::CString>>,
    pub autocmd_fname_full: std::cell::Cell<bool>,
    pub autocmd_bufnr: std::cell::Cell<i32>,
    pub autocmd_match: CloneCell<Option<std::ffi::CString>>,
}

impl AutocmdGlobalState {
    pub const fn new() -> Self {
        Self {
            last_cursormoved_win: CloneCell::new(WinPtr::NULL),
            last_cursormoved: std::cell::Cell::new(CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            }),
            did_cursorhold: std::cell::Cell::new(true),
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
        }
    }

    #[inline]
    pub fn defer_free_buf(&self, buf: BufPtr) {
        self.au_pending_free_buf.borrow_mut().push(buf);
    }

    #[inline]
    pub fn defer_free_win(&self, win: WinPtr) {
        self.au_pending_free_win.borrow_mut().push(win);
    }

    pub fn clear_pending_frees(&self) {
        let bufs = std::mem::take(&mut *self.au_pending_free_buf.borrow_mut());
        let wins = std::mem::take(&mut *self.au_pending_free_win.borrow_mut());
        drop((bufs, wins));
    }
}

pub struct SearchGlobalState {
    pub search_global: SearchState,
    screen_search_hl: std::cell::RefCell<SynMatch>,
    pub search_hl_has_cursor_lnum: std::cell::Cell<Linenr>,
    pub sub_nsubs: std::cell::Cell<i32>,
    pub sub_nlines: std::cell::Cell<Linenr>,
    pub g_do_tagpreview: std::cell::Cell<i32>,
    pub g_tag_at_cursor: std::cell::Cell<bool>,
}

impl SearchGlobalState {
    pub const fn new() -> Self {
        Self {
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
            sub_nsubs: std::cell::Cell::new(0),
            sub_nlines: std::cell::Cell::new(0),
            g_do_tagpreview: std::cell::Cell::new(0),
            g_tag_at_cursor: std::cell::Cell::new(false),
        }
    }

    #[inline]
    pub fn with_screen_search_hl<R>(&self, f: impl FnOnce(&SynMatch) -> R) -> R {
        f(&self.screen_search_hl.borrow())
    }

    #[inline]
    pub fn with_screen_search_hl_mut<R>(&self, f: impl FnOnce(&mut SynMatch) -> R) -> R {
        f(&mut self.screen_search_hl.borrow_mut())
    }
}

pub struct EditGlobalState {
    pub state_global: std::cell::Cell<i32>,
    pub last_mode: std::cell::Cell<[u8; MODE_MAX_LENGTH]>,
    pub dollar_vcol: std::cell::Cell<Colnr>,
    pub edit_submode: std::cell::Cell<Option<&'static std::ffi::CStr>>,
    pub edit_submode_pre: std::cell::Cell<Option<&'static std::ffi::CStr>>,
    pub edit_submode_extra: CloneCell<Option<std::ffi::CString>>,
    pub edit_submode_highl: std::cell::Cell<Hlf>,
    pub visual_global: VisualState,
    pub km_stopsel: std::cell::Cell<bool>,
    pub km_startsel: std::cell::Cell<bool>,
    pub virtual_op: std::cell::Cell<TriState>,
    pub where_paste_started: std::cell::Cell<CursorPos>,
    pub old_indent: std::cell::Cell<i32>,
    pub saved_cursor: std::cell::Cell<CursorPos>,
    pub ins_global: InsState,
    pub orig_line_count: std::cell::Cell<Linenr>,
    pub vr_lines_changed: std::cell::Cell<i32>,
    pub inhibit_delete_count: std::cell::Cell<i32>,
    pub finish_op: std::cell::Cell<bool>,
    pub opcount: std::cell::Cell<i32>,
    pub motion_force: std::cell::Cell<i32>,
    pub no_u_sync: std::cell::Cell<i32>,
    pub u_sync_once: std::cell::Cell<i32>,
    pub force_restart_edit: std::cell::Cell<bool>,
    pub restart_edit: std::cell::Cell<i32>,
    pub ins_at_eol: std::cell::Cell<bool>,
    pub no_abbr: std::cell::Cell<bool>,
    pub replace_offset: std::cell::Cell<i32>,
    last_insert_ga: std::cell::RefCell<Vec<u8>>,
    pub add_last_insert: std::cell::Cell<i32>,
}

impl EditGlobalState {
    pub const fn new() -> Self {
        Self {
            state_global: std::cell::Cell::new(MODE_NORMAL),
            last_mode: std::cell::Cell::new([b'n', 0, 0, 0]),
            dollar_vcol: std::cell::Cell::new(-1),
            edit_submode: std::cell::Cell::new(None),
            edit_submode_pre: std::cell::Cell::new(None),
            edit_submode_extra: CloneCell::new(None),
            edit_submode_highl: std::cell::Cell::new(0),
            visual_global: VisualState {
                start: std::cell::Cell::new(CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                }),
                active: std::cell::Cell::new(false),
                select: std::cell::Cell::new(false),
                select_reg: std::cell::Cell::new(0),
                select_exclu_adj: std::cell::Cell::new(false),
                restart_select: std::cell::Cell::new(0),
                need_end: std::cell::Cell::new(false),
                reselect: std::cell::Cell::new(0),
                mode: std::cell::Cell::new(b'v' as i32),
                resel: crate::normal::VisualExtent {
                    mode: std::cell::Cell::new(0),
                    line_count: std::cell::Cell::new(0),
                    vcol: std::cell::Cell::new(0),
                },
            },
            km_stopsel: std::cell::Cell::new(false),
            km_startsel: std::cell::Cell::new(false),
            virtual_op: std::cell::Cell::new(-1),
            where_paste_started: std::cell::Cell::new(CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            }),
            old_indent: std::cell::Cell::new(0),
            saved_cursor: std::cell::Cell::new(CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            }),
            ins_global: InsState {
                start: std::cell::Cell::new(CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                }),
                start_orig: std::cell::Cell::new(CursorPos {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                }),
                start_textlen: std::cell::Cell::new(0),
                start_blank_vcol: std::cell::Cell::new(0),
                moved: std::cell::Cell::new(0),
                stop_insert_mode: std::cell::Cell::new(false),
                can_cindent: std::cell::Cell::new(false),
                need_undo: std::cell::Cell::new(false),
                did_ai: std::cell::Cell::new(false),
                ai_col: std::cell::Cell::new(0),
                end_comment_pending: std::cell::Cell::new(0),
                did_si: std::cell::Cell::new(false),
                can_si: std::cell::Cell::new(false),
                can_si_back: std::cell::Cell::new(false),
                update_start_orig: std::cell::Cell::new(false),
                new_insert_skip: std::cell::Cell::new(0),
                did_restart_edit: std::cell::Cell::new(0),
                revins_on: std::cell::Cell::new(false),
                revins_chars: std::cell::Cell::new(0),
                revins_legal: std::cell::Cell::new(0),
                revins_scol: std::cell::Cell::new(0),
                dont_sync_undo: std::cell::Cell::new(crate::extmark::K_NONE),
                o_lnum: std::cell::Cell::new(0),
            },
            orig_line_count: std::cell::Cell::new(0),
            vr_lines_changed: std::cell::Cell::new(0),
            inhibit_delete_count: std::cell::Cell::new(0),
            finish_op: std::cell::Cell::new(false),
            opcount: std::cell::Cell::new(0),
            motion_force: std::cell::Cell::new(0),
            no_u_sync: std::cell::Cell::new(0),
            u_sync_once: std::cell::Cell::new(0),
            force_restart_edit: std::cell::Cell::new(false),
            restart_edit: std::cell::Cell::new(0),
            ins_at_eol: std::cell::Cell::new(false),
            no_abbr: std::cell::Cell::new(true),
            replace_offset: std::cell::Cell::new(0),
            last_insert_ga: std::cell::RefCell::new(Vec::new()),
            add_last_insert: std::cell::Cell::new(0),
        }
    }

    #[inline]
    pub fn append_last_insert(&self, s: &[u8]) {
        if !s.is_empty() {
            self.last_insert_ga.borrow_mut().extend_from_slice(s);
        }
    }

    #[inline]
    pub fn clear_last_insert(&self) {
        let prev = std::mem::take(&mut *self.last_insert_ga.borrow_mut());
        drop(prev);
    }

    #[inline]
    pub fn with_last_insert<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let borrow = self.last_insert_ga.borrow();
        f(&borrow)
    }
}

pub struct EditorState {
    pub workspace: WorkspaceState,
    pub msg: MessageGlobalState,
    pub exc: ExceptionGlobalState,
    pub screen: ScreenGlobalState,
    pub input: InputGlobalState,
    pub cmdline: CmdlineGlobalState,
    pub autocmd: AutocmdGlobalState,
    pub search: SearchGlobalState,
    pub edit: EditGlobalState,
}

impl EditorState {
    pub const fn new() -> Self {
        Self {
            workspace: WorkspaceState::new(),
            msg: MessageGlobalState::new(),
            exc: ExceptionGlobalState::new(),
            screen: ScreenGlobalState::new(),
            input: InputGlobalState::new(),
            cmdline: CmdlineGlobalState::new(),
            autocmd: AutocmdGlobalState::new(),
            search: SearchGlobalState::new(),
            edit: EditGlobalState::new(),
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
    g_st().workspace.curwin()
}

#[inline(always)]
pub fn set_curwin(wp: impl Into<WinPtr>) {
    g_st().workspace.set_curwin(wp);
}

#[inline(always)]
pub fn curbuf() -> BufPtr {
    g_st().workspace.curbuf.get()
}

#[inline(always)]
pub fn curbuf_opt() -> Option<BufPtr> {
    let b = g_st().workspace.curbuf.get();
    (!b.is_null()).then_some(b)
}

#[inline(always)]
pub fn curtab() -> TabpagePtr {
    g_st().workspace.curtab.get().clone()
}

#[inline(always)]
pub fn firstwin() -> WinPtr {
    g_st().workspace.firstwin.get()
}

#[inline(always)]
pub fn lastwin() -> WinPtr {
    g_st().workspace.lastwin.get()
}

#[inline(always)]
pub fn prevwin_opt() -> Option<WinPtr> {
    let w = g_st().workspace.prevwin.get();
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
    BufPtrIter(g_st().workspace.firstbuf.get())
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
    BufRevPtrIter(g_st().workspace.lastbuf.get())
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
    WinPtrIter(g_st().workspace.firstwin.get())
}

#[inline(always)]
pub fn win_iter_ptr(start: impl Into<WinPtr>) -> WinPtrIter {
    WinPtrIter(start.into())
}

#[inline(always)]
pub fn for_all_windows_in_tab_ptr(tp: &TabPageInfo) -> WinPtrIter {
    WinPtrIter(if g_st().workspace.curtab.get() == tp {
        g_st().workspace.firstwin.get()
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
    WinRevPtrIter(g_st().workspace.lastwin.get())
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
    TabPtrIter(g_st().workspace.first_tabpage.get().clone())
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
    crate::profile::has_time_fd()
}
#[inline(always)]
pub fn has_redir_fd() -> bool {
    g_st().msg.has_redir_fd()
}
#[inline(always)]
pub fn has_scriptout() -> bool {
    g_st().input.has_scriptout()
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
    g_st().workspace.topframe.get().clone()
}

#[inline(always)]
pub fn first_tabpage() -> TabpagePtr {
    g_st().workspace.first_tabpage.get().clone()
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
            assert_eq!({ g_st().screen.rows_global.get() }, 24);
            assert_eq!({ g_st().screen.columns_global.get() }, 80);
            assert_eq!({ g_st().edit.dollar_vcol.get() }, -1);
            assert_eq!({ g_st().screen.t_colors.get() }, 256);
            assert_eq!(starting_val(), 2);
            assert_eq!({ g_st().edit.state_global.get() }, 0x01);
            assert_eq!({ g_st().edit.visual_global.mode.get() }, b'v' as i32);
            assert_eq!({ g_st().search.search_global.last_line.get() }, 0x7fffffff);
            assert_eq!(NO_LINES_MSG.to_bytes(), b"--No lines in buffer--");
        }
    }

    #[test]
    fn subsystem_domain_methods_preserve_invariants() {
        let _guard = TEST_GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let ws = WorkspaceState::new();
        assert!(ws.curwin().is_null());
        assert!(ws.find_buffer(999_999).is_none());
        assert!(ws.find_window(999_999).is_none());
        assert!(ws.find_tabpage(999_999).is_none());
        ws.register_buffer(999_999, BufPtr::NULL);
        assert!(ws.find_buffer(999_999).is_none());
        ws.unregister_buffer(999_999);
        ws.clear_handles();

        let au = AutocmdGlobalState::new();
        au.defer_free_buf(BufPtr::NULL);
        au.defer_free_win(WinPtr::NULL);
        au.clear_pending_frees();

        let exc = ExceptionGlobalState::new();
        assert!(exc.peek_caught().is_none());
        assert!(exc.pop_caught().is_none());
        assert!(exc.take_caught_stack().is_empty());
        exc.set_caught_stack(Vec::new());
        assert!(!exc.has_msg_list());
        exc.set_msg_list(Some(Vec::new()));
        assert!(exc.has_msg_list());
        assert!(!exc.has_nonempty_msg_list());
        assert!(exc.take_pending_messages().is_none());
        exc.clear_msg_list();
        assert!(exc.take_msg_list().is_some());

        let (slot, is_null) = ws.acquire_ctx_win_slot();
        assert_eq!(slot, 0);
        assert!(is_null);
        assert!(ws.activate_ctx_win(slot).is_null());
        ws.deactivate_ctx_win(slot);
        ws.clear_ctx_wins();
        assert_eq!(ws.ctx_win_len(), 0);

        let msg = MessageGlobalState::new();
        assert!(!msg.is_capturing());
        msg.set_capture(Some(Vec::new()));
        assert!(msg.is_capturing());
        msg.append_capture_byte(b'a');
        msg.append_capture_slice(b"bc");
        assert_eq!(msg.take_capture().as_deref(), Some(&b"abc"[..]));

        let cmd = CmdlineGlobalState::new();
        cmd.set_cmod_flags(0x04);
        cmd.add_cmod_flags(0x08);
        assert_eq!(cmd.cmod_flags(), 0x0c);
        cmd.remove_cmod_flags(0x04);
        assert_eq!(cmd.cmod_flags(), 0x08);
        cmd.set_cmod_split(2);
        cmd.add_cmod_split(1);
        assert_eq!(cmd.cmod_split(), 3);
        cmd.set_cmod_tab(5);
        assert_eq!(cmd.cmod_tab(), 5);

        let edit = EditGlobalState::new();
        edit.append_last_insert(b"xyz");
        edit.with_last_insert(|b| assert_eq!(b, b"xyz"));
        edit.clear_last_insert();
        edit.with_last_insert(|b| assert!(b.is_empty()));

        edit.visual_global.active.set(true);
        edit.visual_global.start.set_lnum(10);
        let snap_vis = edit.visual_global.clone();
        edit.visual_global.active.set(false);
        edit.visual_global.start.set_lnum(1);
        edit.visual_global.restore_from(&snap_vis);
        assert!(edit.visual_global.active.get());
        assert_eq!(edit.visual_global.start.get().lnum, 10);

        let scr = ScreenGlobalState::new();
        scr.with_linebuf_mut(|lb| {
            lb.chars = vec![1, 2];
            lb.attrs = vec![3, 4];
            lb.vcols = vec![5, 6];
        });
        scr.with_linebuf(|lb| {
            assert_eq!(lb.chars, [1, 2]);
            assert_eq!(lb.attrs, [3, 4]);
            assert_eq!(lb.vcols, [5, 6]);
        });
        scr.push_win_extmark(WinExtmark {
            ns_id: 1,
            mark_id: 2,
            win_row: 3,
            win_col: 4,
        });
        assert_eq!(scr.win_extmarks_len(), 1);
        assert_eq!(scr.win_extmark_at(0).mark_id, 2);
        scr.clear_win_extmarks();
        assert_eq!(scr.win_extmarks_len(), 0);

        scr.with_tab_page_click_defs_mut(|defs| {
            defs.push(StlClickDefinition {
                type_: crate::mouse::K_STL_CLICK_DISABLED,
                tabnr: 0,
                func: None,
            });
        });
        scr.set_tab_page_click_def(
            0,
            StlClickDefinition {
                type_: crate::mouse::K_STL_CLICK_TAB_SWITCH,
                tabnr: 2,
                func: None,
            },
        );
        assert_eq!(scr.with_tab_page_click_defs(|defs| defs[0].tabnr), 2);
        scr.clear_tab_page_click_defs();
        assert!(scr.with_tab_page_click_defs(<[StlClickDefinition]>::is_empty));

        scr.with_decor_state_mut(|ds| ds.row = 17);
        let prev_ds = scr.replace_decor_state(DecorState::new());
        assert_eq!(prev_ds.row, 17);
        assert_eq!(scr.with_decor_state(|ds| ds.row), 0);
        scr.set_decor_state(prev_ds);
        assert_eq!(scr.with_decor_state(|ds| ds.row), 17);

        let srch = SearchGlobalState::new();
        srch.with_screen_search_hl_mut(|shl| shl.lnum = 23);
        assert_eq!(srch.with_screen_search_hl(|shl| shl.lnum), 23);

        let inp = InputGlobalState::new();
        assert!(!inp.has_scriptout());
        inp.set_scriptout(None);
        assert!(inp.take_scriptout().is_none());

        assert!(!cmd.cmod_has_filter());
        assert_eq!(cmd.snapshot_cmdmod().cmod_tab, 5);
    }
}
