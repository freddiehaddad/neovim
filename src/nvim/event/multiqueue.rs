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

enum QueueEntry {
    Event(Event),
    Child(crate::globals::GraphPtr<MultiQueue>),
}

#[repr(C)]
pub struct MultiQueue {
    parent: crate::globals::GraphPtr<MultiQueue>,
    items: VecDeque<QueueEntry>,
    notify_loop: crate::globals::GraphPtr<crate::event::r#loop::Loop>,
    size: usize,
    arena_idx: usize,
}

struct MultiQueueState {
    arena: Vec<Option<Box<MultiQueue>>>,
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

fn multiqueue_new_inner(
    parent: Option<&mut MultiQueue>,
    notify_loop: Option<&mut crate::event::r#loop::Loop>,
) -> &'static mut MultiQueue {
    let mut mq = Box::new(MultiQueue {
        parent: crate::globals::GraphPtr::from(parent),
        items: VecDeque::new(),
        notify_loop: crate::globals::GraphPtr::from(notify_loop),
        size: 0,
        arena_idx: 0,
    });
    let mq_ptr = crate::globals::GraphPtr::from(&mut *mq);
    MQ_STATE.with(|cell| {
        let mut st = cell.borrow_mut();
        if let Some(idx) = st.free_slots.pop() {
            mq.arena_idx = idx;
            st.arena[idx] = Some(mq);
        } else {
            let idx = st.arena.len();
            mq.arena_idx = idx;
            st.arena.push(Some(mq));
        }
    });
    mq_ptr.borrow_mut()
}

/// Create a new root (parentless) `MultiQueue` (`multiqueue_new`).
pub fn multiqueue_new(
    notify_loop: Option<&mut crate::event::r#loop::Loop>,
) -> &'static mut MultiQueue {
    multiqueue_new_inner(None, notify_loop)
}

/// Create a new child `MultiQueue` attached to `parent` (`multiqueue_new_child`).
pub fn multiqueue_new_child(parent: &mut MultiQueue) -> &'static mut MultiQueue {
    assert!(parent.parent.is_null());
    parent.size = parent.size.wrapping_add(1);
    multiqueue_new_inner(Some(parent), None)
}

impl Drop for MultiQueue {
    fn drop(&mut self) {
        let self_ptr = crate::globals::GraphPtr::<MultiQueue>::of(&self)
            .as_ptr()
            .cast_const();
        if !self.parent.is_null() && !self.items.is_empty() {
            self.parent
                .items
                .retain(|entry| !matches!(entry, &QueueEntry::Child(child) if child == self_ptr));
        }
    }
}

/// Free a `MultiQueue` and all queued items (`multiqueue_free`).
pub fn multiqueue_free(this: &mut MultiQueue) {
    let idx = this.arena_idx;
    let removed = MQ_STATE.with(|cell| {
        let mut st = cell.borrow_mut();
        st.free_slots.push(idx);
        st.arena[idx].take()
    });
    drop(removed);
}

fn multiqueue_remove(this: &mut MultiQueue) -> Event {
    assert!(!multiqueue_empty(this));
    {
        let q = this;
        let entry = q.items.pop_front().unwrap();
        q.size = q.size.wrapping_sub(1);
        match entry {
            QueueEntry::Child(linked) => {
                assert!(q.parent.is_null());
                let linked_ref = linked.borrow_mut();
                assert!(!multiqueue_empty(linked_ref));
                match linked_ref.items.pop_front().unwrap() {
                    QueueEntry::Event(ev) => ev,
                    QueueEntry::Child(_) => unreachable!(),
                }
            }
            QueueEntry::Event(ev) => {
                let q_ptr = crate::globals::GraphPtr::<MultiQueue>::from_ref(&*q);
                if let Some(parent) = q.parent.opt_mut()
                    && let Some(pos) = parent
                        .items
                        .iter()
                        .position(|e| matches!(e, &QueueEntry::Child(child) if child == q_ptr))
                {
                    parent.items.remove(pos);
                }
                ev
            }
        }
    }
}

fn multiqueue_push(this: &mut MultiQueue, event: Event) {
    let q = this;
    let q_ptr = crate::globals::GraphPtr::<MultiQueue>::from_ref(&*q);
    q.items.push_back(QueueEntry::Event(event));
    if let Some(parent) = q.parent.opt_mut() {
        parent.items.push_back(QueueEntry::Child(q_ptr));
    }
    q.size = q.size.wrapping_add(1);
}

/// Remove the next item and return its `Event` (`multiqueue_get`).
///
/// # Safety
/// `this` must be a valid pointer to a `MultiQueue`.
pub fn multiqueue_get(this: &mut MultiQueue) -> Event {
    if multiqueue_empty(this) {
        Event::nil()
    } else {
        multiqueue_remove(this)
    }
}

/// Enqueue `event` into `this` (`multiqueue_put_event`).
pub fn multiqueue_put_event(this: &mut MultiQueue, event: Event) {
    multiqueue_push(this, event);
    let parent = this.parent;
    if let Some(notify_loop) = parent.opt_mut().and_then(|p| p.notify_loop.opt_mut()) {
        crate::event::r#loop::loop_on_put(notify_loop);
    }
}

/// Move all events from `src` to `dest` (`multiqueue_move_events`).
pub fn multiqueue_move_events(dest: &mut MultiQueue, src: &mut MultiQueue) {
    while !multiqueue_empty(src) {
        let event = multiqueue_get(src);
        multiqueue_put_event(dest, event);
    }
}

/// Process all events in `this` (`multiqueue_process_events`).
pub fn multiqueue_process_events(this: &mut MultiQueue) {
    while !multiqueue_empty(this) {
        let event = multiqueue_remove(this);
        event.dispatch();
    }
}

/// Remove all events from `this` without invoking handlers (`multiqueue_purge_events`).
pub fn multiqueue_purge_events(this: &mut MultiQueue) {
    while !multiqueue_empty(this) {
        let _ = multiqueue_remove(this);
    }
}

/// Return true if `this` has no queued items (`multiqueue_empty`).
pub fn multiqueue_empty(this: &MultiQueue) -> bool {
    std::mem::ManuallyDrop::new(unsafe { std::ptr::read_volatile(&raw const this.items) })
        .is_empty()
}

/// Replace the parent of an empty queue `this` (`multiqueue_replace_parent`).
pub fn multiqueue_replace_parent(this: &mut MultiQueue, new_parent: &mut MultiQueue) {
    assert!(multiqueue_empty(this));
    this.parent = crate::globals::GraphPtr::from(new_parent);
}

/// Return the size counter of `this` (`multiqueue_size`).
pub fn multiqueue_size(this: &MultiQueue) -> usize {
    this.size
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
        let c1 = multiqueue_new_child(&mut *parent);
        let c2 = multiqueue_new_child(&mut *parent);

        let make_ev = Event::Test;
        let ev_val = |ev: Event| match ev {
            Event::Test(v) => v,
            _ => panic!("expected Event::Test"),
        };

        multiqueue_put_event(&mut *c1, make_ev(11));
        multiqueue_put_event(&mut *c2, make_ev(21));
        multiqueue_put_event(&mut *c1, make_ev(12));
        assert_eq!(multiqueue_size(&*c1), 2);
        assert_eq!(multiqueue_size(&*c2), 1);

        // Pop from c2 first; should also unlink from parent.
        assert_eq!(ev_val(multiqueue_get(&mut *c2)), 21);
        assert_eq!(multiqueue_size(&*c2), 0);

        // Remaining in parent should be 11 then 12.
        assert_eq!(ev_val(multiqueue_get(&mut *parent)), 11);
        assert_eq!(ev_val(multiqueue_get(&mut *parent)), 12);
        assert!(multiqueue_empty(&*parent));

        // Child free unlinks remaining events from parent.
        let c3 = multiqueue_new_child(&mut *parent);
        multiqueue_put_event(&mut *c1, make_ev(101));
        multiqueue_put_event(&mut *c2, make_ev(201));
        multiqueue_put_event(&mut *c3, make_ev(301));
        multiqueue_free(c2);
        assert_eq!(ev_val(multiqueue_get(&mut *parent)), 101);
        assert_eq!(ev_val(multiqueue_get(&mut *c3)), 301);
        assert!(multiqueue_empty(&*parent));

        multiqueue_free(c1);
        multiqueue_free(c3);
        multiqueue_free(parent);
    }
}
