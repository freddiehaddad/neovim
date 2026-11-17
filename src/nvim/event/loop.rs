//! Main-thread libuv state and the independently synchronized notification inbox.

pub use crate::event::ffi::*;
use crate::event::inbox::Inbox;
use crate::event::multiqueue::{
    Event, MultiQueue, multiqueue_cancel_events, multiqueue_free, multiqueue_new,
    multiqueue_new_child, multiqueue_process_events, multiqueue_purge_events, multiqueue_put_event,
};
use crate::event::owned::RawHandle;
use crate::event::proc::Proc;
use crate::log::{LOGLVL_ERR, log_uv_handles, logmsg};
use crate::msgpack_rpc::channel::Channel;
use crate::os::time::os_hrtime;
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::ffi::{CString, c_void};
use std::rc::Rc;
use std::sync::Arc;

pub enum ThreadEvent {
    LuaError {
        msg: Option<CString>,
        kind: isize,
    },
    LuaPrint(Vec<u8>),
    #[cfg(windows)]
    PtyFinish(u64),
}

enum Notification {
    Local,
    Remote(ThreadEvent),
}

struct AsyncWake(*mut uv_async_t);

// Only uv_async_send uses this pointer. Inbox serializes it against shutdown,
// and the main loop's storage remains allocated for the process lifetime.
unsafe impl Send for AsyncWake {}

type ThreadInbox = Inbox<Notification, AsyncWake>;
type CloseOwnedHandle = unsafe fn(*mut c_void);

#[derive(Clone)]
pub struct ThreadSender(Arc<ThreadInbox>);

impl ThreadSender {
    pub fn send(&self, event: ThreadEvent) -> bool {
        self.0.send(Notification::Remote(event), wake).is_ok()
    }
}

pub struct Loop {
    uv: RawHandle<192>,
    initialized: Cell<bool>,
    pub events: Cell<MultiQueue>,
    pub fast_events: Cell<MultiQueue>,
    local_events: RefCell<VecDeque<Event>>,
    inbox: OnceCell<Arc<ThreadInbox>>,
    children: RefCell<Vec<Proc>>,
    channels: RefCell<BTreeMap<u64, Rc<Channel>>>,
    owned_handles: RefCell<BTreeMap<usize, CloseOwnedHandle>>,
    children_watcher: RawHandle<40>,
    children_kill_timer: RawHandle<32>,
    poll_timer: RawHandle<32>,
    exit_delay_timer: RawHandle<32>,
    async_: RawHandle<32>,
    recursive: Cell<i32>,
    pub closing: Cell<bool>,
    timeout_expired: Cell<bool>,
}

impl Loop {
    const fn new() -> Self {
        Self {
            uv: RawHandle::new(),
            initialized: Cell::new(false),
            events: Cell::new(MultiQueue::NULL),
            fast_events: Cell::new(MultiQueue::NULL),
            local_events: RefCell::new(VecDeque::new()),
            inbox: OnceCell::new(),
            children: RefCell::new(Vec::new()),
            channels: RefCell::new(BTreeMap::new()),
            owned_handles: RefCell::new(BTreeMap::new()),
            children_watcher: RawHandle::new(),
            children_kill_timer: RawHandle::new(),
            poll_timer: RawHandle::new(),
            exit_delay_timer: RawHandle::new(),
            async_: RawHandle::new(),
            recursive: Cell::new(0),
            closing: Cell::new(false),
            timeout_expired: Cell::new(false),
        }
    }

    pub fn find_channel(&self, id: u64) -> Option<Rc<Channel>> {
        self.channels.borrow().get(&id).cloned()
    }

    pub fn uv_ptr(&self) -> *mut uv_loop_t {
        assert!(self.initialized.get(), "libuv loop is not initialized");
        self.uv.as_ptr()
    }

    pub fn thread_sender(&self) -> ThreadSender {
        ThreadSender(self.inbox.get().expect("loop is not initialized").clone())
    }

    pub fn insert_channel(&self, channel: Rc<Channel>) {
        let previous = self.channels.borrow_mut().insert(channel.id, channel);
        assert!(previous.is_none());
    }

    pub fn remove_channel(&self, id: u64) -> Option<Rc<Channel>> {
        self.channels.borrow_mut().remove(&id)
    }

    pub fn channels(&self) -> Vec<Rc<Channel>> {
        self.channels.borrow().values().cloned().collect()
    }

    pub fn clear_channels(&self) {
        // Requests can retain their channel through its own queue or a shared resize event.
        let owners = self.channels();
        for channel in &owners {
            multiqueue_cancel_events(channel.events);
        }
        let channels = std::mem::take(&mut *self.channels.borrow_mut());
        drop(channels);
    }

    pub fn children(&self) -> Vec<Proc> {
        self.children.borrow().clone()
    }

    pub fn add_child(&self, proc: Proc) {
        self.children.borrow_mut().push(proc);
    }

    pub fn remove_child(&self, proc: &Proc) {
        let removed = {
            let mut children = self.children.borrow_mut();
            let index = children.iter().position(|p| p.ptr_eq(proc)).unwrap();
            children.remove(index)
        };
        drop(removed);
    }

    pub fn has_children(&self) -> bool {
        !self.children.borrow().is_empty()
    }

    pub fn is_polling(&self) -> bool {
        self.recursive.get() != 0
    }

    pub fn register_handle(&self, handle: *mut c_void, close: CloseOwnedHandle) {
        self.owned_handles.borrow_mut().insert(handle.addr(), close);
    }

    pub fn unregister_handle(&self, handle: *mut c_void) {
        self.owned_handles.borrow_mut().remove(&handle.addr());
    }
}

thread_local! {
    static MAIN_LOOP: &'static Loop = Box::leak(Box::new(Loop::new()));
}

pub fn main_loop() -> &'static Loop {
    MAIN_LOOP.with(|loop_| *loop_)
}

pub fn loop_queues(loop_: &Loop) -> &Loop {
    loop_
}

pub fn main_loop_events() -> MultiQueue {
    main_loop().events.get()
}

pub fn loop_children_watcher(loop_: &Loop) -> *mut uv_signal_t {
    assert!(loop_.initialized.get());
    loop_.children_watcher.as_ptr()
}

pub fn loop_children_kill_timer(loop_: &Loop) -> *mut uv_timer_t {
    assert!(loop_.initialized.get());
    loop_.children_kill_timer.as_ptr()
}

pub fn loop_exit_delay_timer(loop_: &Loop) -> *mut uv_timer_t {
    assert!(loop_.initialized.get());
    loop_.exit_delay_timer.as_ptr()
}

pub fn loop_init(loop_: &'static Loop) {
    assert!(loop_.inbox.get().is_none(), "loop is already initialized");
    unsafe {
        assert!(uv_loop_size() <= size_of::<RawHandle<192>>());
        let uv = loop_.uv.as_ptr();
        assert!(!loop_.initialized.get());
        assert_eq!(uv_loop_init(uv), 0);
        loop_.initialized.set(true);
        uv.cast::<*const Loop>().write(loop_);
        loop_.recursive.set(0);
        loop_.closing.set(false);
        loop_.events.set(multiqueue_new(Some(loop_)));
        loop_
            .fast_events
            .set(multiqueue_new_child(loop_.events.get()));
        assert_eq!(uv_async_init(uv, loop_.async_.as_ptr(), Some(async_cb)), 0);
        let inbox = Arc::new(Inbox::new(AsyncWake(loop_.async_.as_ptr())));
        assert!(loop_.inbox.set(inbox).is_ok());
        uv_signal_init(uv, loop_children_watcher(loop_));
        uv_timer_init(uv, loop_children_kill_timer(loop_));
        uv_timer_init(uv, loop_.poll_timer.as_ptr());
        uv_timer_init(uv, loop_exit_delay_timer(loop_));
        loop_.timeout_expired.set(false);
        loop_
            .poll_timer
            .as_ptr()
            .cast::<*const Cell<bool>>()
            .write(&loop_.timeout_expired);
    }
}

fn loop_uv_run(loop_: &Loop, ms: i64) -> bool {
    assert!(loop_.initialized.get());
    assert_eq!(loop_.recursive.replace(1), 0, "recursive uv_run");
    loop_.timeout_expired.set(false);
    unsafe {
        if ms > 0 {
            uv_timer_start(
                loop_.poll_timer.as_ptr(),
                Some(timer_cb),
                ms as u64,
                ms as u64,
            );
        }
        uv_run(
            loop_.uv.as_ptr(),
            if ms == 0 { UV_RUN_NOWAIT } else { UV_RUN_ONCE },
        );
        if ms > 0 {
            uv_timer_stop(loop_.poll_timer.as_ptr());
        }
    }
    loop_.recursive.set(0);
    loop_.timeout_expired.get()
}

pub fn loop_poll_events(loop_: &Loop, ms: i64) -> bool {
    let fast_events = loop_.fast_events.get();
    let expired = loop_uv_run(loop_, ms);
    multiqueue_process_events(fast_events);
    expired
}

fn wake(handle: &AsyncWake) {
    unsafe { uv_async_send(handle.0) };
}

pub fn loop_schedule_fast(loop_: &Loop, event: Event) {
    if loop_.closing.get() {
        return;
    }
    assert!(loop_.initialized.get());
    loop_.local_events.borrow_mut().push_back(event);
    if loop_
        .inbox
        .get()
        .is_none_or(|inbox| inbox.send(Notification::Local, wake).is_err())
    {
        let event = loop_.local_events.borrow_mut().pop_back();
        drop(event);
    }
}

pub fn loop_schedule_deferred(loop_: &Loop, event: Event) {
    loop_schedule_fast(loop_, Event::LoopDeferred(Box::new(event)));
}

pub fn loop_deferred_event(loop_: &Loop, event: Event) {
    multiqueue_put_event(loop_.events.get(), event);
}

pub fn loop_on_put(loop_: &Loop) {
    if loop_.recursive.get() != 0 {
        unsafe { uv_stop(loop_.uv.as_ptr()) };
    }
}

unsafe extern "C" fn loop_walk_cb(handle: *mut uv_handle_t, _arg: *mut c_void) {
    if unsafe { uv_is_closing(handle) } != 0 {
        return;
    }
    let close = main_loop()
        .owned_handles
        .borrow()
        .get(&handle.addr())
        .copied();
    if let Some(close) = close {
        unsafe { close(handle) };
    } else {
        unsafe { uv_close(handle, None) };
    }
}

pub fn loop_close(loop_: &Loop, wait: bool) -> bool {
    if loop_.closing.replace(true) {
        return !loop_.initialized.get();
    }
    if let Some(inbox) = loop_.inbox.get() {
        drop(inbox.close());
    }
    let local = std::mem::take(&mut *loop_.local_events.borrow_mut());
    drop(local);
    unsafe {
        uv_close(loop_children_watcher(loop_), None);
        uv_close(loop_children_kill_timer(loop_), None);
        uv_close(loop_.poll_timer.as_ptr(), None);
        uv_close(loop_exit_delay_timer(loop_), None);
        uv_close(loop_.async_.as_ptr(), None);
    }
    let start = if wait { os_hrtime() } else { 0 };
    let mut didstop = false;
    let mut success = true;
    loop {
        unsafe {
            uv_run(
                loop_.uv.as_ptr(),
                if didstop {
                    UV_RUN_DEFAULT
                } else {
                    UV_RUN_NOWAIT
                },
            );
        }
        let close_status = unsafe { uv_loop_close(loop_.uv.as_ptr()) };
        if close_status != UV_EBUSY || !wait {
            if close_status == 0 {
                loop_.initialized.set(false);
            }
            break;
        }
        if (os_hrtime() - start) / NS_PER_SEC >= 2 {
            success = false;
            logmsg!(
                LOGLVL_ERR,
                None,
                Some(c"loop_close"),
                line!() as i32,
                true,
                c"uv_loop_close() hang?"
            );
            log_uv_handles(unsafe { &mut *loop_.uv.as_ptr() });
            break;
        }
        if !didstop {
            unsafe {
                uv_stop(loop_.uv.as_ptr());
                uv_walk(loop_.uv.as_ptr(), Some(loop_walk_cb), std::ptr::null_mut());
            }
            didstop = true;
        }
    }
    multiqueue_free(loop_.fast_events.replace(MultiQueue::NULL));
    multiqueue_free(loop_.events.replace(MultiQueue::NULL));
    let children = std::mem::take(&mut *loop_.children.borrow_mut());
    drop(children);
    success
}

pub fn loop_purge(loop_: &Loop) {
    if let Some(inbox) = loop_.inbox.get() {
        drop(inbox.take());
    }
    let local = std::mem::take(&mut *loop_.local_events.borrow_mut());
    drop(local);
    multiqueue_purge_events(loop_.fast_events.get());
}

pub fn loop_size(loop_: &Loop) -> usize {
    loop_.inbox.get().map_or(0, |inbox| inbox.len())
}

unsafe extern "C" fn async_cb(_handle: *mut uv_async_t) {
    let loop_ = main_loop();
    for notification in loop_.inbox.get().unwrap().take() {
        let event = match notification {
            Notification::Local => loop_.local_events.borrow_mut().pop_front().unwrap(),
            Notification::Remote(ThreadEvent::LuaError { msg, kind }) => {
                Event::LoopDeferred(Box::new(Event::NluaLuvError { msg, kind }))
            }
            Notification::Remote(ThreadEvent::LuaPrint(bytes)) => {
                Event::LoopDeferred(Box::new(Event::NluaPrint(
                    crate::api::private::validate::NvimString::from_bytes(&bytes),
                )))
            }
            #[cfg(windows)]
            Notification::Remote(ThreadEvent::PtyFinish(id)) => {
                let Some(channel) = loop_.find_channel(id) else {
                    continue;
                };
                Event::PtyProcFinish(channel.uv.proc_.clone())
            }
        };
        multiqueue_put_event(loop_.fast_events.get(), event);
    }
}

unsafe extern "C" fn timer_cb(handle: *mut uv_timer_t) {
    unsafe { (&*handle.cast::<*const Cell<bool>>().read()).set(true) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::libuv_proc::UvProcessHandle;
    use crate::event::signal::SignalWatcher;
    use crate::event::socket::UvSocketHandle;
    use crate::event::stream::{UV_NAMED_PIPE, UV_TCP, UV_TTY, UvStreamHandle};
    use crate::event::timer::TIMER_HANDLE_WORDS;

    #[test]
    fn channel_cleanup_cancels_late_native_requests() {
        unsafe extern "C" fn late_read(handle: *mut uv_timer_t) {
            let ids = unsafe { &**handle.cast::<*const [u64; 2]>() };
            let requests: [&[u8]; 2] = [
                b"\x94\x00\x01\xa9nvim_eval\x91\xa11",
                b"\x94\x00\x01\xb2nvim_ui_try_resize\x92\x50\x18",
            ];
            for (&id, request) in ids.iter().zip(requests) {
                let channel = crate::msgpack_rpc::channel::find_channel_opt(id).unwrap();
                assert_eq!(
                    crate::msgpack_rpc::channel::receive_msgpack(request, &channel, false),
                    request.len()
                );
                crate::channel::channel_decref(&channel);
            }
        }

        let _lock = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let loop_ = main_loop();
        loop_init(loop_);
        let resize_events = multiqueue_new_child(loop_.events.get());
        crate::globals::screen_state()
            .resize_events
            .set(resize_events);
        let channels: [Rc<Channel>; 2] = std::array::from_fn(|_| {
            crate::channel::channel_alloc(crate::api::vim::K_CHANNEL_STREAM_INTERNAL)
        });
        for channel in &channels {
            crate::msgpack_rpc::channel::rpc_start(channel);
            channel.rpc.closed.set(true);
            crate::channel::channel_decref(channel);
        }
        let queues = channels.each_ref().map(|channel| channel.events);
        let weak = channels.each_ref().map(Rc::downgrade);
        let context = Box::new(channels.each_ref().map(|channel| channel.id));
        drop(channels);
        assert!(crate::event::multiqueue::multiqueue_empty(
            loop_.events.get()
        ));

        let timer = Box::new(RawHandle::<32>::new());
        unsafe {
            assert_eq!(uv_timer_init(loop_.uv_ptr(), timer.as_ptr()), 0);
            timer.as_ptr().cast::<*const [u64; 2]>().write(&*context);
            assert_eq!(uv_timer_start(timer.as_ptr(), Some(late_read), 0, 0), 0);
        }
        assert!(loop_close(loop_, true));
        crate::msgpack_rpc::channel::rpc_free_all_mem();
        crate::channel::channel_free_all_mem();
        let retained = weak.each_ref().map(|weak| weak.upgrade().is_some());

        crate::globals::screen_state()
            .resize_events
            .set(MultiQueue::NULL);
        multiqueue_process_events(resize_events);
        multiqueue_free(resize_events);
        for queue in queues {
            multiqueue_free(queue);
        }
        assert_eq!(retained, [false, false]);
    }

    #[test]
    fn cached_clock_tracks_native_loop_storage() {
        let _lock = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _editor = crate::main_init::test_editor();
        let loop_ = main_loop();
        let before = unsafe { uv_now(loop_.uv_ptr()) };
        loop_poll_events(loop_, 1);
        assert!(crate::os::time::os_now() >= before);
        assert_eq!(crate::os::time::os_now(), unsafe { uv_now(loop_.uv_ptr()) });
    }

    #[test]
    fn notification_shutdown_finishes_the_native_wake_before_close() {
        unsafe extern "C" fn notified(_handle: *mut uv_async_t) {}

        let loop_storage = Box::new(RawHandle::<192>::new());
        let async_storage = Box::new(RawHandle::<32>::new());
        unsafe {
            assert_eq!(uv_loop_init(loop_storage.as_ptr()), 0);
            assert_eq!(
                uv_async_init(
                    loop_storage.as_ptr(),
                    async_storage.as_ptr(),
                    Some(notified)
                ),
                0
            );
        }
        let inbox = Arc::new(Inbox::new(AsyncWake(async_storage.as_ptr())));
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            let sender = inbox.clone();
            let sender_entered = entered.clone();
            let sender_release = release.clone();
            scope.spawn(move || {
                sender
                    .send(17, |handle| {
                        sender_entered.wait();
                        sender_release.wait();
                        assert_eq!(unsafe { uv_async_send(handle.0) }, 0);
                    })
                    .unwrap();
            });
            entered.wait();
            scope.spawn(|| release.wait());
            assert_eq!(inbox.close().into_iter().collect::<Vec<_>>(), vec![17]);
        });
        unsafe {
            uv_close(async_storage.as_ptr(), None);
            uv_run(loop_storage.as_ptr(), UV_RUN_DEFAULT);
            assert_eq!(uv_loop_close(loop_storage.as_ptr()), 0);
        }
        assert_eq!(
            inbox.send(23, |_| panic!("wake after native close")),
            Err(23)
        );
    }

    #[test]
    fn uv_handle_and_loop_storage_sizes_are_sufficient() {
        unsafe {
            let loop_dummy = Loop::new();
            assert!(size_of_val(&loop_dummy.uv) >= uv_loop_size());
            assert!(size_of_val(&loop_dummy.children_watcher) >= uv_handle_size(16));
            assert!(size_of_val(&loop_dummy.children_kill_timer) >= uv_handle_size(13));
            assert!(size_of_val(&loop_dummy.poll_timer) >= uv_handle_size(13));
            assert!(size_of_val(&loop_dummy.exit_delay_timer) >= uv_handle_size(13));
            assert!(size_of_val(&loop_dummy.async_) >= uv_handle_size(1));
            assert!(size_of_val(&SignalWatcher::new().uv) >= uv_handle_size(16));
            assert!(size_of::<RawHandle<TIMER_HANDLE_WORDS>>() >= uv_handle_size(13));
            let stream_size = uv_handle_size(UV_NAMED_PIPE)
                .max(uv_handle_size(UV_TCP))
                .max(uv_handle_size(UV_TTY))
                .max(uv_handle_size(6));
            assert!(size_of::<UvStreamHandle>() >= stream_size);
            assert!(size_of::<UvSocketHandle>() >= stream_size);
            assert!(size_of::<UvProcessHandle>() >= uv_handle_size(10));
        }
    }
}
