//! Multi-level queue for selective async event processing (`src/nvim/event/multiqueue.c`).

use std::ffi::{c_int, c_void};

use crate::memory::{xfree, xmalloc};

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

#[repr(C)]
struct MultiQueueItem {
    link: bool,
    queue: *mut MultiQueue,
    event: Event,
    parent_item: *mut MultiQueueItem,
    prev: *mut MultiQueueItem,
    next: *mut MultiQueueItem,
}

#[repr(C)]
pub struct MultiQueue {
    parent: *mut MultiQueue,
    head: *mut MultiQueueItem,
    tail: *mut MultiQueueItem,
    on_put: PutCallback,
    data: *mut c_void,
    size: usize,
}

struct MulticastEvent {
    event: Event,
    fired: bool,
    refcount: c_int,
}

#[inline]
unsafe fn push_tail(q: &mut MultiQueue, item: *mut MultiQueueItem) {
    unsafe {
        (*item).prev = q.tail;
        (*item).next = std::ptr::null_mut();
        if !q.tail.is_null() {
            (*q.tail).next = item;
        } else {
            q.head = item;
        }
        q.tail = item;
    }
}

#[inline]
unsafe fn unlink(q: &mut MultiQueue, item: *mut MultiQueueItem) {
    unsafe {
        let prev = (*item).prev;
        let next = (*item).next;
        if !prev.is_null() {
            (*prev).next = next;
        } else {
            q.head = next;
        }
        if !next.is_null() {
            (*next).prev = prev;
        } else {
            q.tail = prev;
        }
        (*item).prev = std::ptr::null_mut();
        (*item).next = std::ptr::null_mut();
    }
}

fn multiqueue_new_inner(
    parent: *mut MultiQueue,
    on_put: PutCallback,
    data: *mut c_void,
) -> *mut MultiQueue {
    let rv = unsafe { xmalloc(std::mem::size_of::<MultiQueue>()).cast::<MultiQueue>() };
    unsafe {
        rv.write(MultiQueue {
            parent,
            head: std::ptr::null_mut(),
            tail: std::ptr::null_mut(),
            on_put,
            data,
            size: 0,
        });
    }
    rv
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
        let mut curr = q.head;
        while !curr.is_null() {
            let next = (*curr).next;
            if !q.parent.is_null() && !(*curr).parent_item.is_null() {
                let pitem = (*curr).parent_item;
                unlink(&mut *q.parent, pitem);
                xfree(pitem.cast::<c_void>());
            }
            unlink(q, curr);
            xfree(curr.cast::<c_void>());
            curr = next;
        }
        xfree(this.cast::<c_void>());
    }
}

unsafe fn multiqueueitem_get_event(
    owner: &mut MultiQueue,
    item: *mut MultiQueueItem,
    remove: bool,
) -> Event {
    assert!(!item.is_null());
    unsafe {
        if (*item).link {
            let linked = (*item).queue;
            assert!(!multiqueue_empty(linked));
            let linked_ref = &mut *linked;
            let child = linked_ref.head;
            let ev = (*child).event;
            if remove {
                unlink(linked_ref, child);
                xfree(child.cast::<c_void>());
            }
            ev
        } else {
            if remove && !(*item).parent_item.is_null() {
                let pitem = (*item).parent_item;
                if !owner.parent.is_null() {
                    unlink(&mut *owner.parent, pitem);
                }
                xfree(pitem.cast::<c_void>());
                (*item).parent_item = std::ptr::null_mut();
            }
            (*item).event
        }
    }
}

unsafe fn multiqueue_remove(this: *mut MultiQueue) -> Event {
    assert!(unsafe { !multiqueue_empty(this) });
    unsafe {
        let q = &mut *this;
        let h = q.head;
        unlink(q, h);
        assert!(!(*h).link || q.parent.is_null());
        let ev = multiqueueitem_get_event(q, h, true);
        q.size = q.size.wrapping_sub(1);
        xfree(h.cast::<c_void>());
        ev
    }
}

unsafe fn multiqueue_push(this: *mut MultiQueue, event: Event) {
    unsafe {
        let q = &mut *this;
        let item = xmalloc(std::mem::size_of::<MultiQueueItem>()).cast::<MultiQueueItem>();
        item.write(MultiQueueItem {
            link: false,
            queue: std::ptr::null_mut(),
            event,
            parent_item: std::ptr::null_mut(),
            prev: std::ptr::null_mut(),
            next: std::ptr::null_mut(),
        });
        push_tail(q, item);

        if !q.parent.is_null() {
            let pitem = xmalloc(std::mem::size_of::<MultiQueueItem>()).cast::<MultiQueueItem>();
            pitem.write(MultiQueueItem {
                link: true,
                queue: this,
                event: Event::nil(),
                parent_item: std::ptr::null_mut(),
                prev: std::ptr::null_mut(),
                next: std::ptr::null_mut(),
            });
            (*item).parent_item = pitem;
            push_tail(&mut *q.parent, pitem);
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
    unsafe { (*this).head.is_null() }
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
            xfree(data.cast::<c_void>());
        }
    }
}

/// Create a one-shot multicast event safe to enqueue on `num` queues (`event_create_oneshot`).
pub fn event_create_oneshot(ev: Event, num: c_int) -> Event {
    let data = unsafe { xmalloc(std::mem::size_of::<MulticastEvent>()).cast::<MulticastEvent>() };
    unsafe {
        data.write(MulticastEvent {
            event: ev,
            fired: false,
            refcount: num,
        });
    }
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
