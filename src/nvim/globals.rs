#![deny(unsafe_code)]

//! Global variables (`src/nvim/globals.h`).

pub use crate::api::private::validate::CloneCell;
pub use crate::autocmd::autocmd_state;
pub use crate::cursor::CursorPosCellExt;
pub use crate::drawscreen::screen_state;
pub use crate::ex_eval::exc_state;
pub use crate::ex_getln::cmdline_state;
pub use crate::input::input_state;
pub use crate::message::msg_state;
pub use crate::normal::edit_state;
pub use crate::search::search_state;

use crate::api::ui::NO_SCREEN;
use crate::buffer::{FileBuf, SEA_NONE};
use crate::context::CtxWin;
use crate::ex_getln::Disptick;
use crate::grid::{GridTarget, ScreenGrid};
use crate::highlight::HLF_COUNT;
use crate::lua::converter::NluaRefState;
use crate::window::{Frame, TabPageInfo, WinInfoInfo};

pub(crate) const IOSIZE: usize = 1024 + 1;
pub(crate) const DFLT_ROWS: i32 = 24;
pub(crate) const DFLT_COLS: i32 = 80;
pub(crate) const PROF_NONE: i32 = 0;

/// RAII guard that adds `delta` to a `Cell<i32>` on creation and subtracts `delta` on `Drop`.
#[must_use = "if unused, the scoped counter will be decremented immediately"]
pub struct ScopedCellDeltaGuard {
    cell: &'static std::cell::Cell<i32>,
    delta: i32,
}

impl ScopedCellDeltaGuard {
    #[inline]
    pub fn new(cell: &'static std::cell::Cell<i32>, delta: i32) -> Self {
        cell.set(cell.get() + delta);
        Self { cell, delta }
    }
}

impl Drop for ScopedCellDeltaGuard {
    #[inline]
    fn drop(&mut self) {
        self.cell.set(self.cell.get() - self.delta);
    }
}

/// RAII guard that replaces a `Cell<T>` with `new_val` and restores the previous value on `Drop`.
#[must_use = "if unused, the previous Cell value will be restored immediately"]
pub struct ScopedCellValueGuard<T: Copy + 'static> {
    cell: &'static std::cell::Cell<T>,
    prev: T,
}

impl<T: Copy + 'static> ScopedCellValueGuard<T> {
    #[inline]
    pub fn new(cell: &'static std::cell::Cell<T>, new_val: T) -> Self {
        let prev = cell.replace(new_val);
        Self { cell, prev }
    }
}

impl<T: Copy + 'static> Drop for ScopedCellValueGuard<T> {
    #[inline]
    fn drop(&mut self) {
        self.cell.set(self.prev);
    }
}

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

    #[inline]
    pub fn disallow_tabpage_move(&'static self) -> ScopedCellDeltaGuard {
        ScopedCellDeltaGuard::new(&self.tabpage_move_disallowed, 1)
    }

    #[inline]
    pub fn pause_fold_update(&'static self) -> ScopedCellDeltaGuard {
        ScopedCellDeltaGuard::new(&self.disable_fold_update, 1)
    }

    #[inline]
    pub fn pause_check_timestamps(&'static self) -> ScopedCellDeltaGuard {
        ScopedCellDeltaGuard::new(&self.no_check_timestamps, 1)
    }
}

thread_local! {
    static WORKSPACE_STATE: &'static WorkspaceState =
        Box::leak(Box::new(WorkspaceState::new()));
}

#[inline(always)]
pub fn workspace_state() -> &'static WorkspaceState {
    WORKSPACE_STATE.with(|s| *s)
}

#[allow(non_upper_case_globals, unsafe_code)]
#[unsafe(export_name = "curwin")]
pub static CURWIN_FFI: std::sync::atomic::AtomicPtr<WinInfoInfo> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[inline(always)]
pub fn curwin() -> WinPtr {
    workspace_state().curwin()
}

#[inline(always)]
pub fn set_curwin(wp: impl Into<WinPtr>) {
    workspace_state().set_curwin(wp);
}

#[inline(always)]
pub fn curbuf() -> BufPtr {
    workspace_state().curbuf.get()
}

#[inline(always)]
pub fn curbuf_opt() -> Option<BufPtr> {
    let b = workspace_state().curbuf.get();
    (!b.is_null()).then_some(b)
}

#[inline(always)]
pub fn curtab() -> TabpagePtr {
    workspace_state().curtab.get().clone()
}

#[inline(always)]
pub fn firstwin() -> WinPtr {
    workspace_state().firstwin.get()
}

#[inline(always)]
pub fn lastwin() -> WinPtr {
    workspace_state().lastwin.get()
}

#[inline(always)]
pub fn prevwin_opt() -> Option<WinPtr> {
    let w = workspace_state().prevwin.get();
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
    BufPtrIter(workspace_state().firstbuf.get())
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
    BufRevPtrIter(workspace_state().lastbuf.get())
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
    WinPtrIter(workspace_state().firstwin.get())
}

#[inline(always)]
pub fn win_iter_ptr(start: impl Into<WinPtr>) -> WinPtrIter {
    WinPtrIter(start.into())
}

#[inline(always)]
pub fn for_all_windows_in_tab_ptr(tp: &TabPageInfo) -> WinPtrIter {
    WinPtrIter(if workspace_state().curtab.get() == tp {
        workspace_state().firstwin.get()
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
    WinRevPtrIter(workspace_state().lastwin.get())
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
    TabPtrIter(workspace_state().first_tabpage.get().clone())
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
    msg_state().has_redir_fd()
}
#[inline(always)]
pub fn has_scriptout() -> bool {
    input_state().has_scriptout()
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
    workspace_state().topframe.get().clone()
}

#[inline(always)]
pub fn first_tabpage() -> TabpagePtr {
    workspace_state().first_tabpage.get().clone()
}

#[inline(always)]
pub fn nlua_global_refs() -> NluaRefState {
    crate::lua::executor::nlua_global_refs()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::autocmd::AutocmdGlobalState;
    use crate::decoration::DecorState;
    use crate::drawscreen::{ScreenGlobalState, WinExtmark};
    use crate::ex_eval::ExceptionGlobalState;
    use crate::ex_getln::CmdlineGlobalState;
    use crate::input::InputGlobalState;
    use crate::message::MessageGlobalState;
    use crate::mouse::StlClickDefinition;
    use crate::normal::EditGlobalState;
    use crate::search::SearchGlobalState;

    #[test]
    fn default_globals_match_c_initializers() {
        let _guard = TEST_GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        {
            assert_eq!({ screen_state().rows_global.get() }, 24);
            assert_eq!({ screen_state().columns_global.get() }, 80);
            assert_eq!({ edit_state().dollar_vcol.get() }, -1);
            assert_eq!({ screen_state().t_colors.get() }, 256);
            assert_eq!(starting_val(), 2);
            assert_eq!({ edit_state().state_global.get() }, 0x01);
            assert_eq!({ edit_state().visual_global.mode.get() }, b'v' as i32);
            assert_eq!({ search_state().search_global.last_line.get() }, 0x7fffffff);
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

    #[test]
    fn scoped_cell_guards_restore_and_nest_correctly() {
        let _guard = TEST_GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(input_state().textlock.get(), 0);
        {
            let _g1 = input_state().lock_text();
            assert_eq!(input_state().textlock.get(), 1);
            {
                let _g2 = input_state().lock_text();
                assert_eq!(input_state().textlock.get(), 2);
                {
                    let _ov = input_state().override_textlock(0);
                    assert_eq!(input_state().textlock.get(), 0);
                }
                assert_eq!(input_state().textlock.get(), 2);
            }
            assert_eq!(input_state().textlock.get(), 1);
        }
        assert_eq!(input_state().textlock.get(), 0);

        assert_eq!(screen_state().redrawing_disabled.get(), 0);
        {
            let _rd1 = screen_state().disable_redrawing();
            assert_eq!(screen_state().redrawing_disabled.get(), 1);
            {
                let _rd2 = screen_state().override_redrawing_disabled(0);
                assert_eq!(screen_state().redrawing_disabled.get(), 0);
            }
            assert_eq!(screen_state().redrawing_disabled.get(), 1);
        }
        assert_eq!(screen_state().redrawing_disabled.get(), 0);

        assert_eq!(msg_state().emsg_off.get(), 0);
        {
            let _e = msg_state().suppress_emsg();
            assert_eq!(msg_state().emsg_off.get(), 1);
        }
        assert_eq!(msg_state().emsg_off.get(), 0);

        assert_eq!(edit_state().no_u_sync.get(), 0);
        {
            let _u = edit_state().disable_u_sync();
            assert_eq!(edit_state().no_u_sync.get(), 1);
        }
        assert_eq!(edit_state().no_u_sync.get(), 0);
    }

    #[test]
    fn cell_backed_graph_fields_roundtrip() {
        let fmark = crate::mark::FmarkCell::new();
        fmark.mark.set_lnum(42);
        fmark.mark.set_col(7);
        fmark.mark.set_coladd(2);
        fmark.fnum.set(3);
        let snap = fmark.get();
        assert_eq!(snap.mark.lnum, 42);
        assert_eq!(snap.mark.col, 7);
        assert_eq!(snap.mark.coladd, 2);
        assert_eq!(snap.fnum, 3);
        fmark.reset(crate::cursor::CursorPos::new(), 0, snap.view);
        assert_eq!(fmark.mark.get().lnum, 0);

        let vis = crate::normal::VisualinfoCell::new();
        vis.vi_start.set_lnum(5);
        vis.vi_end.set_lnum(10);
        vis.vi_mode.set(b'V' as i32);
        vis.vi_curswant.set(99);
        let vsnap = vis.get();
        assert_eq!(vsnap.vi_start.lnum, 5);
        assert_eq!(vsnap.vi_end.lnum, 10);
        assert_eq!(vsnap.vi_mode, b'V' as i32);
        assert_eq!(vsnap.vi_curswant, 99);

        let ts = crate::buffer::TabstopArray::from_vec(vec![4, 8, 2]);
        assert!(!ts.is_empty());
        assert_eq!(&*ts, &[4, 8, 2]);
        assert!(crate::buffer::TabstopArray::EMPTY.is_empty());

        let lcs = crate::window::LcsCharsCell::new();
        let mut lcs_val = crate::window::LcsChars::new();
        lcs_val.tab1 = b'>' as u32;
        lcs_val.multispace = vec![b'.' as u32, b':' as u32];
        lcs_val.leadmultispace = vec![b'+' as u32];
        lcs.set(lcs_val);
        assert_eq!(lcs.tab1.get(), b'>' as u32);
        assert!(lcs.has_multispace());
        assert_eq!(lcs.multispace_len(), 2);
        assert_eq!(lcs.multispace_at(0), b'.' as u32);
        assert_eq!(lcs.multispace_at(1), b':' as u32);
        assert!(lcs.has_leadmultispace());
        assert_eq!(lcs.leadmultispace_len(), 1);
        assert_eq!(lcs.leadmultispace_at(0), b'+' as u32);
        lcs.clear_multispace();
        assert!(!lcs.has_multispace());
        assert!(!lcs.has_leadmultispace());
    }
}
