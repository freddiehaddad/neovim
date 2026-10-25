//! Multi-level queue for selective async event processing (`src/nvim/event/multiqueue.c`).

use std::collections::VecDeque;
use std::ffi::{c_int, c_void};

pub const EVENT_HANDLER_MAX_ARGC: usize = 10;

pub type argv_callback = Option<unsafe fn(argv: *mut *mut c_void)>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Event {
    pub handler: argv_callback,
    pub argv: [*mut c_void; EVENT_HANDLER_MAX_ARGC],
}

impl Event {
    pub const fn nil() -> Self {
        Self {
            handler: None,
            argv: [std::ptr::null_mut(); EVENT_HANDLER_MAX_ARGC],
        }
    }
}

pub type PutCallback = Option<unsafe fn(multiq: *mut MultiQueue, data: *mut c_void)>;

#[derive(Clone, Copy)]
enum QueueEntry {
    Event(Event),
    Child(*mut MultiQueue),
}

#[repr(C)]
pub struct MultiQueue {
    parent: *mut MultiQueue,
    items: VecDeque<QueueEntry>,
    on_put: PutCallback,
    data: *mut c_void,
    size: usize,
}

struct MulticastEvent {
    event: Event,
    fired: bool,
    refcount: c_int,
}

fn multiqueue_new_inner(
    parent: *mut MultiQueue,
    on_put: PutCallback,
    data: *mut c_void,
) -> *mut MultiQueue {
    Box::into_raw(Box::new(MultiQueue {
        parent,
        items: VecDeque::new(),
        on_put,
        data,
        size: 0,
    }))
}

/// Create a new root (parentless) `MultiQueue` (`multiqueue_new`).
pub fn multiqueue_new(on_put: PutCallback, data: *mut c_void) -> *mut MultiQueue {
    multiqueue_new_inner(std::ptr::null_mut(), on_put, data)
}

/// Create a new child `MultiQueue` attached to `parent` (`multiqueue_new_child`).
///
/// # Safety
/// `parent` must be a valid pointer to a root `MultiQueue` (`parent->parent == NULL`).
pub unsafe fn multiqueue_new_child(parent: *mut MultiQueue) -> *mut MultiQueue {
    let parent_ref = unsafe { &mut *parent };
    assert!(parent_ref.parent.is_null());
    parent_ref.size = parent_ref.size.wrapping_add(1);
    multiqueue_new_inner(parent, None, std::ptr::null_mut())
}

/// Free a `MultiQueue` and all queued items (`multiqueue_free`).
///
/// # Safety
/// `this` must be a valid pointer to a `MultiQueue`.
pub unsafe fn multiqueue_free(this: *mut MultiQueue) {
    assert!(!this.is_null());
    unsafe {
        let q = &mut *this;
        if !q.parent.is_null() && !q.items.is_empty() {
            (*q.parent)
                .items
                .retain(|entry| !matches!(*entry, QueueEntry::Child(child) if child == this));
        }
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
                        .position(|e| matches!(*e, QueueEntry::Child(child) if child == this))
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
pub unsafe fn multiqueue_get(this: *mut MultiQueue) -> Event {
    unsafe {
        if multiqueue_empty(this) {
            Event::nil()
        } else {
            multiqueue_remove(this)
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
        if !parent.is_null()
            && let Some(on_put) = (*parent).on_put
        {
            on_put(parent, (*parent).data);
        }
    }
}

/// Move all events from `src` to `dest` (`multiqueue_move_events`).
///
/// # Safety
/// `dest` and `src` must be valid pointers to `MultiQueue`s.
pub unsafe fn multiqueue_move_events(dest: *mut MultiQueue, src: *mut MultiQueue) {
    unsafe {
        while !multiqueue_empty(src) {
            let event = multiqueue_get(src);
            multiqueue_put_event(dest, event);
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
            let mut event = multiqueue_remove(this);
            if let Some(handler) = event.handler {
                handler(event.argv.as_mut_ptr());
            }
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
pub unsafe fn multiqueue_replace_parent(this: *mut MultiQueue, new_parent: *mut MultiQueue) {
    assert!(unsafe { multiqueue_empty(this) });
    unsafe {
        (*this).parent = new_parent;
    }
}

/// Return the size counter of `this` (`multiqueue_size`).
///
/// # Safety
/// `this` must be a valid pointer to a `MultiQueue`.
pub unsafe fn multiqueue_size(this: *mut MultiQueue) -> usize {
    unsafe { (*this).size }
}

unsafe fn multiqueue_oneshot_event(argv: *mut *mut c_void) {
    unsafe {
        let data = (*argv).cast::<MulticastEvent>();
        if !(*data).fired {
            (*data).fired = true;
            if let Some(handler) = (*data).event.handler {
                handler((*data).event.argv.as_mut_ptr());
            }
        }
        (*data).refcount -= 1;
        if (*data).refcount == 0 {
            drop(Box::from_raw(data));
        }
    }
}

/// Create a one-shot multicast event safe to enqueue on `num` queues (`event_create_oneshot`).
pub fn event_create_oneshot(ev: Event, num: c_int) -> Event {
    let data = Box::into_raw(Box::new(MulticastEvent {
        event: ev,
        fired: false,
        refcount: num,
    }));
    let mut out = Event::nil();
    out.handler = Some(multiqueue_oneshot_event);
    out.argv[0] = data.cast::<c_void>();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_and_child_interleaved_ordering() {
        unsafe {
            let parent = multiqueue_new(None, std::ptr::null_mut());
            let c1 = multiqueue_new_child(parent);
            let c2 = multiqueue_new_child(parent);

            let make_ev = |v: usize| {
                let mut ev = Event::nil();
                ev.argv[0] = v as *mut c_void;
                ev
            };

            multiqueue_put_event(c1, make_ev(11));
            multiqueue_put_event(c2, make_ev(21));
            multiqueue_put_event(c1, make_ev(12));
            assert_eq!(multiqueue_size(c1), 2);
            assert_eq!(multiqueue_size(c2), 1);

            // Pop from c2 first; should also unlink from parent.
            assert_eq!(multiqueue_get(c2).argv[0] as usize, 21);
            assert_eq!(multiqueue_size(c2), 0);

            // Remaining in parent should be 11 then 12.
            assert_eq!(multiqueue_get(parent).argv[0] as usize, 11);
            assert_eq!(multiqueue_get(parent).argv[0] as usize, 12);
            assert!(multiqueue_empty(parent));

            // Child free unlinks remaining events from parent.
            let c3 = multiqueue_new_child(parent);
            multiqueue_put_event(c1, make_ev(101));
            multiqueue_put_event(c2, make_ev(201));
            multiqueue_put_event(c3, make_ev(301));
            multiqueue_free(c2);
            assert_eq!(multiqueue_get(parent).argv[0] as usize, 101);
            assert_eq!(multiqueue_get(c3).argv[0] as usize, 301);
            assert!(multiqueue_empty(parent));

            multiqueue_free(c1);
            multiqueue_free(c3);
            multiqueue_free(parent);
        }
    }
}
