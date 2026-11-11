//! Multi-level queue for selective async event processing (`src/nvim/event/multiqueue.c`).

use std::cell::Cell;
use std::collections::VecDeque;
use std::ffi::CString;
use std::rc::Rc;

pub enum Event {
    None,
    Oneshot(Rc<Cell<Option<Box<Event>>>>),
    LoopDeferred {
        loop_: crate::globals::GraphPtr<crate::event::r#loop::Loop>,
        event: Box<Event>,
    },
    TimeWatcher(crate::globals::GraphPtr<crate::event::time::TimeWatcher>),
    TimeWatcherClose(crate::globals::GraphPtr<crate::event::time::TimeWatcher>),
    Signal(crate::globals::GraphPtr<crate::event::signal::SignalWatcher>),
    RStreamRead(crate::globals::GraphPtr<crate::event::rstream::RStream>),
    SocketConnection {
        watcher: crate::globals::GraphPtr<crate::event::socket::SocketWatcher>,
        status: i32,
    },
    ProcCloseHandles(crate::globals::GraphPtr<crate::event::proc::Proc>),
    ProcClose(crate::globals::GraphPtr<crate::event::proc::Proc>),
    ProcExit(i32),
    AutoCmdDeferred(Box<crate::autocmd::AutoCmdEvent>),
    AutoCmdOptionSetModified {
        bufnr: i32,
        new_val: bool,
    },
    AutoCmdVimResume,
    ChannelFree(crate::globals::GraphPtr<crate::channel::Channel>),
    ChannelCallback(crate::globals::GraphPtr<crate::channel::Channel>),
    ChannelTermDelayedFree(crate::globals::GraphPtr<crate::channel::Channel>),
    ChannelSetInfo {
        chan: crate::globals::GraphPtr<crate::channel::Channel>,
        event: crate::autocmd::AuEvent,
    },
    RpcRequest(Box<crate::msgpack_rpc::channel::RequestEvent>),
    RpcInternalRead {
        channel: crate::globals::GraphPtr<crate::channel::Channel>,
        buffer: Rc<crate::event::wstream::WBuffer>,
    },
    RpcClose(crate::globals::GraphPtr<crate::channel::Channel>),
    NluaLuvError {
        msg: Option<CString>,
        kind: isize,
    },
    NluaSchedule {
        cb: crate::lua::converter::LuaRef,
        ns_id: u32,
    },
    NluaPrint(crate::api::private::validate::NvimString),
    MsgSemsg(CString),
    MsgSemsgMultiline(CString),
    CursorHold,
    #[cfg(windows)]
    PtyProcFinish(crate::globals::GraphPtr<crate::event::proc::Proc>),
    ShellOutData {
        str_copy: Vec<u8>,
        fd: i32,
    },
    TermRequest(Box<crate::terminal::TermRequestEvent>),
    TermStateChange(crate::msgpack_rpc::packer::ObjHandle),
    TermSyncFlush(crate::msgpack_rpc::packer::ObjHandle),
    TermClipboardSet {
        mask: i32,
        data: CString,
    },
    TInputDone,
    UiRefresh,
    UiClientConnect(CString),
    CmdwinOpen {
        firstc: i32,
        content: Option<CString>,
        pos: i32,
    },
    #[cfg(test)]
    Test(usize),
}

impl Event {
    pub const fn nil() -> Self {
        Self::None
    }

    pub fn dispatch(self) {
        match self {
            Self::None => {}
            Self::Oneshot(data) => {
                if let Some(ev) = data.take() {
                    ev.dispatch();
                }
            }
            Self::LoopDeferred { loop_, event } => {
                crate::event::r#loop::loop_deferred_event(loop_.borrow_mut(), *event);
            }
            Self::TimeWatcher(watcher) => crate::event::time::time_event(watcher.borrow_mut()),
            Self::TimeWatcherClose(watcher) => {
                crate::event::time::close_event(watcher.borrow_mut())
            }
            Self::Signal(watcher) => crate::event::signal::signal_event(watcher.borrow_mut()),
            Self::RStreamRead(stream) => crate::event::rstream::read_event(stream.borrow_mut()),
            Self::SocketConnection { watcher, status } => {
                crate::event::socket::connection_event(watcher.borrow_mut(), status);
            }
            Self::ProcCloseHandles(proc_) => {
                crate::event::proc::proc_close_handles(proc_.borrow_mut())
            }
            Self::ProcClose(proc_) => crate::event::proc::proc_close_event(proc_.borrow_mut()),
            Self::ProcExit(status) => crate::event::proc::exit_event(status),
            Self::AutoCmdDeferred(ev) => crate::autocmd::deferred_event(*ev),
            Self::AutoCmdOptionSetModified { bufnr, new_val } => {
                crate::autocmd::deferred_optionset_modified(bufnr, new_val);
            }
            Self::AutoCmdVimResume => crate::autocmd::vimresume_event(),
            Self::ChannelFree(chan) => crate::channel::free_channel_event(chan.opt_mut()),
            Self::ChannelCallback(chan) => crate::channel::on_channel_event(chan.borrow_mut()),
            Self::ChannelTermDelayedFree(chan) => {
                crate::channel::term_delayed_free(chan.borrow_mut())
            }
            Self::ChannelSetInfo { chan, event } => {
                crate::channel::set_info_event(chan.borrow_mut(), event);
            }
            Self::RpcRequest(ev) => crate::msgpack_rpc::channel::request_event(*ev),
            Self::RpcInternalRead { channel, buffer } => {
                crate::msgpack_rpc::channel::internal_read_event(channel.borrow_mut(), buffer);
            }
            Self::RpcClose(chan) => crate::msgpack_rpc::channel::rpc_close_event(chan.opt_mut()),
            Self::NluaLuvError { msg, kind } => {
                crate::lua::executor::nlua_luv_error_event(msg, kind);
            }
            Self::NluaSchedule { cb, ns_id } => {
                crate::lua::executor::nlua_schedule_event(cb, ns_id);
            }
            Self::NluaPrint(text) => {
                crate::lua::executor::nlua_print_event(text);
            }
            Self::MsgSemsg(s) => crate::message::msg_semsg_event(s),
            Self::MsgSemsgMultiline(s) => crate::message::msg_semsg_multiline_event(s),
            Self::CursorHold => crate::os::input::cursorhold_event(),
            #[cfg(windows)]
            Self::PtyProcFinish(proc_) => {
                crate::os::pty_proc_win::pty_proc_finish(proc_.borrow_mut())
            }
            Self::ShellOutData { str_copy, fd } => {
                crate::os::shell::out_data_event(str_copy, fd);
            }
            Self::TermRequest(ev) => crate::terminal::emit_termrequest(ev),
            Self::TermStateChange(buf_handle) => {
                crate::terminal::terminal_state_change_event(buf_handle);
            }
            Self::TermSyncFlush(buf_handle) => crate::terminal::on_sync_flush(buf_handle),
            Self::TermClipboardSet { mask, data } => {
                crate::terminal::term_clipboard_set(mask, data);
            }
            Self::TInputDone => crate::tui::input::tinput_done_event(),
            Self::UiRefresh => crate::ui::ui_refresh_event(),
            Self::UiClientConnect(addr) => crate::ui_client::channel_connect_event(addr),
            Self::CmdwinOpen {
                firstc,
                content,
                pos,
            } => crate::ex_getln::open_cmdwin_event(firstc, content, pos),
            #[cfg(test)]
            Self::Test(_) => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MultiQueue(u32);

impl MultiQueue {
    pub const NULL: Self = Self(0);

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    #[inline(always)]
    pub const fn borrow(self) -> Self {
        self
    }

    #[inline(always)]
    pub const fn borrow_mut(self) -> Self {
        self
    }

    #[inline(always)]
    pub const fn opt(self) -> Option<Self> {
        if self.0 == 0 { None } else { Some(self) }
    }

    #[inline(always)]
    pub const fn opt_mut(self) -> Option<Self> {
        self.opt()
    }
}

enum QueueEntry {
    Event(Event),
    Child(MultiQueue),
}

struct MultiQueueInner {
    parent: MultiQueue,
    items: VecDeque<QueueEntry>,
    notify_main_loop: bool,
    size: usize,
}

struct MultiQueueState {
    arena: Vec<Option<MultiQueueInner>>,
    free_slots: Vec<usize>,
}

impl MultiQueueState {
    const fn new() -> Self {
        Self {
            arena: Vec::new(),
            free_slots: Vec::new(),
        }
    }
}

thread_local! {
    static MQ_STATE: std::cell::RefCell<MultiQueueState> = const {
        std::cell::RefCell::new(MultiQueueState::new())
    };
}

fn multiqueue_new_inner(parent: MultiQueue, notify_main_loop: bool) -> MultiQueue {
    let mq = MultiQueueInner {
        parent,
        items: VecDeque::new(),
        notify_main_loop,
        size: 0,
    };
    MQ_STATE.with(|cell| {
        let mut st = cell.borrow_mut();
        if let Some(idx) = st.free_slots.pop() {
            st.arena[idx] = Some(mq);
            MultiQueue((idx + 1) as u32)
        } else {
            let idx = st.arena.len();
            st.arena.push(Some(mq));
            MultiQueue((idx + 1) as u32)
        }
    })
}

/// Create a new root (parentless) `MultiQueue` (`multiqueue_new`).
pub fn multiqueue_new(notify_loop: Option<&mut crate::event::r#loop::Loop>) -> MultiQueue {
    multiqueue_new_inner(MultiQueue::NULL, notify_loop.is_some())
}

/// Create a new child `MultiQueue` attached to `parent` (`multiqueue_new_child`).
pub fn multiqueue_new_child(parent: MultiQueue) -> MultiQueue {
    MQ_STATE.with(|cell| {
        let mut st = cell.borrow_mut();
        let p = st.arena[(parent.0 - 1) as usize].as_mut().unwrap();
        assert!(p.parent.is_null());
        p.size = p.size.wrapping_add(1);
    });
    multiqueue_new_inner(parent, false)
}

/// Free a `MultiQueue` and all queued items (`multiqueue_free`).
pub fn multiqueue_free(this: MultiQueue) {
    if this.is_null() {
        return;
    }
    let idx = (this.0 - 1) as usize;
    let removed = MQ_STATE.with(|cell| {
        let mut st = cell.borrow_mut();
        let removed = st.arena[idx].take();
        if let Some(ref q) = removed {
            st.free_slots.push(idx);
            if !q.parent.is_null()
                && !q.items.is_empty()
                && let Some(Some(parent)) = st.arena.get_mut((q.parent.0 - 1) as usize)
            {
                parent
                    .items
                    .retain(|entry| !matches!(entry, &QueueEntry::Child(child) if child == this));
            }
        }
        removed
    });
    drop(removed);
}

fn multiqueue_remove(this: MultiQueue) -> Event {
    assert!(!multiqueue_empty(this));
    MQ_STATE.with(|cell| {
        let mut st = cell.borrow_mut();
        let (entry, parent) = {
            let q = st.arena[(this.0 - 1) as usize].as_mut().unwrap();
            let entry = q.items.pop_front().unwrap();
            q.size = q.size.wrapping_sub(1);
            (entry, q.parent)
        };
        match entry {
            QueueEntry::Child(linked) => {
                assert!(parent.is_null());
                let linked_ref = st.arena[(linked.0 - 1) as usize].as_mut().unwrap();
                assert!(!linked_ref.items.is_empty());
                match linked_ref.items.pop_front().unwrap() {
                    QueueEntry::Event(ev) => ev,
                    QueueEntry::Child(_) => unreachable!(),
                }
            }
            QueueEntry::Event(ev) => {
                if !parent.is_null()
                    && let Some(Some(parent_q)) = st.arena.get_mut((parent.0 - 1) as usize)
                    && let Some(pos) = parent_q
                        .items
                        .iter()
                        .position(|e| matches!(e, &QueueEntry::Child(child) if child == this))
                {
                    parent_q.items.remove(pos);
                }
                ev
            }
        }
    })
}

fn multiqueue_push(this: MultiQueue, event: Event) -> bool {
    MQ_STATE.with(|cell| {
        let mut st = cell.borrow_mut();
        let parent = {
            let q = st.arena[(this.0 - 1) as usize].as_mut().unwrap();
            q.items.push_back(QueueEntry::Event(event));
            q.size = q.size.wrapping_add(1);
            q.parent
        };
        if !parent.is_null()
            && let Some(Some(parent_q)) = st.arena.get_mut((parent.0 - 1) as usize)
        {
            parent_q.items.push_back(QueueEntry::Child(this));
            parent_q.notify_main_loop
        } else {
            false
        }
    })
}

/// Remove the next item and return its `Event` (`multiqueue_get`).
pub fn multiqueue_get(this: MultiQueue) -> Event {
    if multiqueue_empty(this) {
        Event::nil()
    } else {
        multiqueue_remove(this)
    }
}

/// Enqueue `event` into `this` (`multiqueue_put_event`).
pub fn multiqueue_put_event(this: MultiQueue, event: Event) {
    if multiqueue_push(this, event) {
        crate::event::r#loop::loop_on_put(&mut crate::globals::g_st().main_loop);
    }
}

/// Move all events from `src` to `dest` (`multiqueue_move_events`).
pub fn multiqueue_move_events(dest: MultiQueue, src: MultiQueue) {
    while !multiqueue_empty(src) {
        let event = multiqueue_get(src);
        multiqueue_put_event(dest, event);
    }
}

/// Process all events in `this` (`multiqueue_process_events`).
pub fn multiqueue_process_events(this: MultiQueue) {
    while !multiqueue_empty(this) {
        let event = multiqueue_remove(this);
        event.dispatch();
    }
}

/// Remove all events from `this` without invoking handlers (`multiqueue_purge_events`).
pub fn multiqueue_purge_events(this: MultiQueue) {
    while !multiqueue_empty(this) {
        let _ = multiqueue_remove(this);
    }
}

/// Return true if `this` has no queued items (`multiqueue_empty`).
pub fn multiqueue_empty(this: MultiQueue) -> bool {
    if this.is_null() {
        return true;
    }
    MQ_STATE.with(|cell| {
        cell.borrow()
            .arena
            .get((this.0 - 1) as usize)
            .and_then(Option::as_ref)
            .is_none_or(|q| q.items.is_empty())
    })
}

/// Replace the parent of an empty queue `this` (`multiqueue_replace_parent`).
pub fn multiqueue_replace_parent(this: MultiQueue, new_parent: MultiQueue) {
    assert!(multiqueue_empty(this));
    MQ_STATE.with(|cell| {
        let mut st = cell.borrow_mut();
        st.arena[(this.0 - 1) as usize].as_mut().unwrap().parent = new_parent;
    });
}

/// Return the size counter of `this` (`multiqueue_size`).
pub fn multiqueue_size(this: MultiQueue) -> usize {
    if this.is_null() {
        return 0;
    }
    MQ_STATE.with(|cell| {
        cell.borrow()
            .arena
            .get((this.0 - 1) as usize)
            .and_then(Option::as_ref)
            .map_or(0, |q| q.size)
    })
}

/// Create a shared one-shot multicast event cell (`event_create_oneshot`).
pub fn event_create_oneshot(ev: Event) -> Rc<Cell<Option<Box<Event>>>> {
    Rc::new(Cell::new(Some(Box::new(ev))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_and_child_interleaved_ordering() {
        let parent = multiqueue_new(None);
        let c1 = multiqueue_new_child(parent);
        let c2 = multiqueue_new_child(parent);

        let make_ev = Event::Test;
        let ev_val = |ev: Event| match ev {
            Event::Test(v) => v,
            _ => panic!("expected Event::Test"),
        };

        multiqueue_put_event(c1, make_ev(11));
        multiqueue_put_event(c2, make_ev(21));
        multiqueue_put_event(c1, make_ev(12));
        assert_eq!(multiqueue_size(c1), 2);
        assert_eq!(multiqueue_size(c2), 1);

        // Pop from c2 first; should also unlink from parent.
        assert_eq!(ev_val(multiqueue_get(c2)), 21);
        assert_eq!(multiqueue_size(c2), 0);

        // Remaining in parent should be 11 then 12.
        assert_eq!(ev_val(multiqueue_get(parent)), 11);
        assert_eq!(ev_val(multiqueue_get(parent)), 12);
        assert!(multiqueue_empty(parent));

        // Child free unlinks remaining events from parent.
        let c3 = multiqueue_new_child(parent);
        multiqueue_put_event(c1, make_ev(101));
        multiqueue_put_event(c2, make_ev(201));
        multiqueue_put_event(c3, make_ev(301));
        multiqueue_free(c2);
        assert_eq!(ev_val(multiqueue_get(parent)), 101);
        assert_eq!(ev_val(multiqueue_get(c3)), 301);
        assert!(multiqueue_empty(parent));

        multiqueue_free(c1);
        multiqueue_free(c3);
        multiqueue_free(parent);
    }
}
