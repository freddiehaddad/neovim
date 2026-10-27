//! Multi-level queue for selective async event processing (`src/nvim/event/multiqueue.c`).

use std::cell::Cell;
use std::collections::VecDeque;
use std::ffi::{CString, c_char, c_int};
use std::rc::Rc;

pub enum Event {
    None,
    Oneshot(Rc<Cell<Option<Box<Event>>>>),
    LoopDeferred {
        loop_: *mut crate::event::r#loop::Loop,
        event: Box<Event>,
    },
    TimeWatcher(*mut crate::event::time::TimeWatcher),
    TimeWatcherClose(*mut crate::event::time::TimeWatcher),
    Signal(*mut crate::event::signal::SignalWatcher),
    RStreamRead(*mut crate::event::rstream::RStream),
    SocketConnection {
        watcher: *mut crate::event::socket::SocketWatcher,
        status: c_int,
    },
    ProcCloseHandles(*mut crate::event::proc::Proc),
    ProcClose(*mut crate::event::proc::Proc),
    ProcExit(c_int),
    AutoCmdDeferred(Box<crate::autocmd::AutoCmdEvent>),
    AutoCmdOptionSetModified {
        bufnr: c_int,
        new_val: bool,
    },
    AutoCmdVimResume,
    ChannelFree(*mut crate::channel::Channel),
    ChannelCallback(*mut crate::channel::Channel),
    ChannelTermDelayedFree(*mut crate::channel::Channel),
    ChannelSetInfo {
        chan: *mut crate::channel::Channel,
        event: crate::autocmd::event_T,
    },
    RpcRequest(Box<crate::msgpack_rpc::channel::RequestEvent>),
    RpcInternalRead {
        channel: *mut crate::channel::Channel,
        buffer: Rc<crate::event::wstream::WBuffer>,
    },
    RpcClose(*mut crate::channel::Channel),
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
    PtyProcFinish(*mut crate::event::proc::Proc),
    ShellOutData {
        str_copy: Vec<c_char>,
        fd: c_int,
    },
    TermRequest(Box<crate::terminal::TermRequestEvent>),
    TermStateChange(crate::msgpack_rpc::packer::handle_T),
    TermSyncFlush(crate::msgpack_rpc::packer::handle_T),
    TermClipboardSet {
        mask: c_int,
        data: CString,
    },
    TInputDone,
    UiRefresh,
    UiClientConnect(CString),
    CmdwinOpen {
        firstc: c_int,
        content: Option<CString>,
        pos: c_int,
    },
    #[cfg(test)]
    Test(usize),
}

impl Event {
    pub const fn nil() -> Self {
        Self::None
    }

    pub unsafe fn dispatch(self) {
        unsafe {
            match self {
                Self::None => {}
                Self::Oneshot(data) => {
                    if let Some(ev) = data.take() {
                        ev.dispatch();
                    }
                }
                Self::LoopDeferred { loop_, event } => {
                    crate::event::r#loop::loop_deferred_event(&mut *loop_, *event);
                }
                Self::TimeWatcher(watcher) => crate::event::time::time_event(&mut *watcher),
                Self::TimeWatcherClose(watcher) => crate::event::time::close_event(&mut *watcher),
                Self::Signal(watcher) => crate::event::signal::signal_event(&mut *watcher),
                Self::RStreamRead(stream) => crate::event::rstream::read_event(&mut *stream),
                Self::SocketConnection { watcher, status } => {
                    crate::event::socket::connection_event(watcher, status);
                }
                Self::ProcCloseHandles(proc_) => crate::event::proc::proc_close_handles(&mut *proc_),
                Self::ProcClose(proc_) => crate::event::proc::proc_close_event(&mut *proc_),
                Self::ProcExit(status) => crate::event::proc::exit_event(status),
                Self::AutoCmdDeferred(ev) => crate::autocmd::deferred_event(*ev),
                Self::AutoCmdOptionSetModified { bufnr, new_val } => {
                    crate::autocmd::deferred_optionset_modified(bufnr, new_val);
                }
                Self::AutoCmdVimResume => crate::autocmd::vimresume_event(),
                Self::ChannelFree(chan) => crate::channel::free_channel_event(&mut *chan),
                Self::ChannelCallback(chan) => crate::channel::on_channel_event(&mut *chan),
                Self::ChannelTermDelayedFree(chan) => crate::channel::term_delayed_free(&mut *chan),
                Self::ChannelSetInfo { chan, event } => {
                    crate::channel::set_info_event(&mut *chan, event);
                }
                Self::RpcRequest(ev) => crate::msgpack_rpc::channel::request_event(*ev),
                Self::RpcInternalRead { channel, buffer } => {
                    crate::msgpack_rpc::channel::internal_read_event(&mut *channel, buffer);
                }
                Self::RpcClose(chan) => crate::msgpack_rpc::channel::rpc_close_event(chan),
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
                Self::PtyProcFinish(proc_) => crate::os::pty_proc_win::pty_proc_finish(&mut *proc_),
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
}

enum QueueEntry {
    Event(Event),
    Child(*mut MultiQueue),
}

#[repr(C)]
pub struct MultiQueue {
    parent: *mut MultiQueue,
    items: VecDeque<QueueEntry>,
    notify_loop: *mut crate::event::r#loop::Loop,
    size: usize,
}

fn multiqueue_new_inner(
    parent: *mut MultiQueue,
    notify_loop: *mut crate::event::r#loop::Loop,
) -> *mut MultiQueue {
    Box::into_raw(Box::new(MultiQueue {
        parent,
        items: VecDeque::new(),
        notify_loop,
        size: 0,
    }))
}

/// Create a new root (parentless) `MultiQueue` (`multiqueue_new`).
pub fn multiqueue_new(notify_loop: *mut crate::event::r#loop::Loop) -> *mut MultiQueue {
    multiqueue_new_inner(std::ptr::null_mut(), notify_loop)
}

/// Create a new child `MultiQueue` attached to `parent` (`multiqueue_new_child`).
///
/// # Safety
/// `parent` must be a valid pointer to a root `MultiQueue` (`parent->parent == NULL`).
pub unsafe fn multiqueue_new_child(parent: *mut MultiQueue) -> *mut MultiQueue {
    let parent_ref = unsafe { &mut *parent };
    assert!(parent_ref.parent.is_null());
    parent_ref.size = parent_ref.size.wrapping_add(1);
    multiqueue_new_inner(parent, std::ptr::null_mut())
}

impl Drop for MultiQueue {
    fn drop(&mut self) {
        unsafe {
            if !self.parent.is_null() && !self.items.is_empty() {
                (*self.parent).items.retain(
                    |entry| !matches!(entry, &QueueEntry::Child(child) if std::ptr::eq(child, self)),
                );
            }
        }
    }
}

/// Free a `MultiQueue` and all queued items (`multiqueue_free`).
///
/// # Safety
/// `this` must be a valid pointer to a `MultiQueue`.
pub unsafe fn multiqueue_free(this: *mut MultiQueue) {
    assert!(!this.is_null());
    unsafe {
        drop(Box::from_raw(this));
    }
}

unsafe fn multiqueue_remove(this: *mut MultiQueue) -> Event {
    assert!(unsafe { !multiqueue_empty(this) });
    unsafe {
        let q = &mut *this;
        let entry = q.items.pop_front().unwrap();
        q.size = q.size.wrapping_sub(1);
        match entry {
            QueueEntry::Child(linked) => {
                assert!(q.parent.is_null());
                assert!(!multiqueue_empty(linked));
                let linked_ref = &mut *linked;
                match linked_ref.items.pop_front().unwrap() {
                    QueueEntry::Event(ev) => ev,
                    QueueEntry::Child(_) => unreachable!(),
                }
            }
            QueueEntry::Event(ev) => {
                if !q.parent.is_null() {
                    let parent = &mut *q.parent;
                    if let Some(pos) = parent
                        .items
                        .iter()
                        .position(|e| matches!(e, &QueueEntry::Child(child) if child == this))
                    {
                        parent.items.remove(pos);
                    }
                }
                ev
            }
        }
    }
}

unsafe fn multiqueue_push(this: *mut MultiQueue, event: Event) {
    unsafe {
        let q = &mut *this;
        q.items.push_back(QueueEntry::Event(event));
        if !q.parent.is_null() {
            (*q.parent).items.push_back(QueueEntry::Child(this));
        }
        q.size = q.size.wrapping_add(1);
    }
}

/// Remove the next item and return its `Event` (`multiqueue_get`).
///
/// # Safety
/// `this` must be a valid pointer to a `MultiQueue`.
pub unsafe fn multiqueue_get(this: &mut MultiQueue) -> Event {
    unsafe {
        if multiqueue_empty(&raw mut *this) {
            Event::nil()
        } else {
            multiqueue_remove(&raw mut *this)
        }
    }
}

/// Enqueue `event` into `this` (`multiqueue_put_event`).
///
/// # Safety
/// `this` must be a valid pointer to a `MultiQueue`.
pub unsafe fn multiqueue_put_event(this: *mut MultiQueue, event: Event) {
    assert!(!this.is_null());
    unsafe {
        multiqueue_push(this, event);
        let parent = (*this).parent;
        if !parent.is_null() && !(*parent).notify_loop.is_null() {
            crate::event::r#loop::loop_on_put(&mut *(*parent).notify_loop);
        }
    }
}

/// Move all events from `src` to `dest` (`multiqueue_move_events`).
///
/// # Safety
/// `dest` and `src` must be valid pointers to `MultiQueue`s.
pub unsafe fn multiqueue_move_events(dest: &mut MultiQueue, src: &mut MultiQueue) {
    unsafe {
        while !multiqueue_empty(&raw mut *src) {
            let event = multiqueue_get(src);
            multiqueue_put_event(&raw mut *dest, event);
        }
    }
}

/// Process all events in `this` (`multiqueue_process_events`).
///
/// # Safety
/// `this` must be a valid pointer to a `MultiQueue`.
pub unsafe fn multiqueue_process_events(this: *mut MultiQueue) {
    assert!(!this.is_null());
    unsafe {
        while !multiqueue_empty(this) {
            let event = multiqueue_remove(this);
            event.dispatch();
        }
    }
}

/// Remove all events from `this` without invoking handlers (`multiqueue_purge_events`).
///
/// # Safety
/// `this` must be a valid pointer to a `MultiQueue`.
pub unsafe fn multiqueue_purge_events(this: *mut MultiQueue) {
    assert!(!this.is_null());
    unsafe {
        while !multiqueue_empty(this) {
            let _ = multiqueue_remove(this);
        }
    }
}

/// Return true if `this` has no queued items (`multiqueue_empty`).
///
/// # Safety
/// `this` must be a valid pointer to a `MultiQueue`.
pub unsafe fn multiqueue_empty(this: *mut MultiQueue) -> bool {
    assert!(!this.is_null());
    unsafe { (*this).items.is_empty() }
}

/// Replace the parent of an empty queue `this` (`multiqueue_replace_parent`).
///
/// # Safety
/// `this` must be a valid pointer to an empty `MultiQueue`.
pub unsafe fn multiqueue_replace_parent(this: &mut MultiQueue, new_parent: &mut MultiQueue) {
    assert!(unsafe { multiqueue_empty(&raw mut *this) });
    this.parent = new_parent;
}

/// Return the size counter of `this` (`multiqueue_size`).
///
/// # Safety
/// `this` must be a valid pointer to a `MultiQueue`.
pub unsafe fn multiqueue_size(this: &mut MultiQueue) -> usize {
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
        unsafe {
            let parent = multiqueue_new(std::ptr::null_mut());
            let c1 = multiqueue_new_child(&mut *parent);
            let c2 = multiqueue_new_child(&mut *parent);

            let make_ev = Event::Test;
            let ev_val = |ev: Event| match ev {
                Event::Test(v) => v,
                _ => panic!("expected Event::Test"),
            };

            multiqueue_put_event(c1, make_ev(11));
            multiqueue_put_event(c2, make_ev(21));
            multiqueue_put_event(c1, make_ev(12));
            assert_eq!(multiqueue_size(&mut *c1), 2);
            assert_eq!(multiqueue_size(&mut *c2), 1);

            // Pop from c2 first; should also unlink from parent.
            assert_eq!(ev_val(multiqueue_get(&mut *c2)), 21);
            assert_eq!(multiqueue_size(&mut *c2), 0);

            // Remaining in parent should be 11 then 12.
            assert_eq!(ev_val(multiqueue_get(&mut *parent)), 11);
            assert_eq!(ev_val(multiqueue_get(&mut *parent)), 12);
            assert!(multiqueue_empty(parent));

            // Child free unlinks remaining events from parent.
            let c3 = multiqueue_new_child(&mut *parent);
            multiqueue_put_event(c1, make_ev(101));
            multiqueue_put_event(c2, make_ev(201));
            multiqueue_put_event(c3, make_ev(301));
            multiqueue_free(c2);
            assert_eq!(ev_val(multiqueue_get(&mut *parent)), 101);
            assert_eq!(ev_val(multiqueue_get(&mut *c3)), 301);
            assert!(multiqueue_empty(parent));

            multiqueue_free(c1);
            multiqueue_free(c3);
            multiqueue_free(parent);
        }
    }
}
