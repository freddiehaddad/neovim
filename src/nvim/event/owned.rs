//! Address-stable storage retained by a registered C callback.

use std::cell::{Cell, UnsafeCell};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::rc::Rc;

pub struct RawHandle<const WORDS: usize>(UnsafeCell<[u64; WORDS]>);

impl<const WORDS: usize> RawHandle<WORDS> {
    pub const fn new() -> Self {
        Self(UnsafeCell::new([0; WORDS]))
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.0.get().cast()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Uninitialized,
    Active,
    Closing,
    Closed,
}

pub struct CallbackHandle<T, const WORDS: usize> {
    storage: UnsafeCell<[u64; WORDS]>,
    phase: Cell<Phase>,
    owner: PhantomData<Rc<T>>,
}

impl<T, const WORDS: usize> CallbackHandle<T, WORDS> {
    pub const fn new() -> Self {
        Self {
            storage: UnsafeCell::new([0; WORDS]),
            phase: Cell::new(Phase::Uninitialized),
            owner: PhantomData,
        }
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.storage.get().cast()
    }

    pub fn can_initialize(&self) -> bool {
        matches!(self.phase.get(), Phase::Uninitialized | Phase::Closed)
    }

    pub fn is_active(&self) -> bool {
        self.phase.get() == Phase::Active
    }

    /// The initialized C handle must be a field of `owner`. Its first word is
    /// libuv's opaque `data` pointer. Only its close callback may release it.
    pub unsafe fn register(&self, owner: Rc<T>) {
        assert!(self.can_initialize());
        unsafe { self.as_ptr().cast::<*const T>().write(Rc::into_raw(owner)) };
        self.phase.set(Phase::Active);
    }

    pub fn begin_close(&self) -> bool {
        if self.phase.get() != Phase::Active {
            return false;
        }
        self.phase.set(Phase::Closing);
        true
    }

    /// `handle` must still hold the registration created by `register`.
    pub unsafe fn callback_owner(handle: *mut c_void) -> Rc<T> {
        let owner = unsafe { handle.cast::<*const T>().read() };
        assert!(!owner.is_null());
        unsafe {
            Rc::increment_strong_count(owner);
            Rc::from_raw(owner)
        }
    }

    /// Call only after C has finished using the handle. Returning the registered
    /// owner defers its destruction until after this borrow of the handle ends.
    pub unsafe fn finish_close(&self) -> Rc<T> {
        assert_eq!(self.phase.get(), Phase::Closing);
        let owner = unsafe { self.as_ptr().cast::<*const T>().replace(std::ptr::null()) };
        self.phase.set(Phase::Closed);
        unsafe { Rc::from_raw(owner) }
    }
}

impl<T, const WORDS: usize> Default for CallbackHandle<T, WORDS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct State {
        handle: CallbackHandle<State, 4>,
        drops: Rc<Cell<usize>>,
        logical_refs: Cell<usize>,
    }

    impl Drop for State {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    #[test]
    fn registration_and_queued_callback_outlive_logical_close() {
        let drops = Rc::new(Cell::new(0));
        let owner = Rc::new(State {
            handle: CallbackHandle::new(),
            drops: drops.clone(),
            logical_refs: Cell::new(1),
        });
        let handle = owner.handle.as_ptr();
        unsafe { owner.handle.register(owner.clone()) };
        let queued = unsafe { CallbackHandle::<State, 4>::callback_owner(handle) };
        owner.logical_refs.set(0);
        assert!(owner.handle.begin_close());
        assert!(!owner.handle.begin_close());
        drop(owner);
        assert_eq!(drops.get(), 0);

        let closing = unsafe { CallbackHandle::<State, 4>::callback_owner(handle) };
        assert_eq!(closing.logical_refs.get(), 0);
        drop(unsafe { closing.handle.finish_close() });
        drop(closing);
        assert_eq!(drops.get(), 0);
        assert_eq!(queued.handle.as_ptr(), handle);
        drop(queued);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn unregistered_failure_does_not_retain_storage() {
        let drops = Rc::new(Cell::new(0));
        let owner = Rc::new(State {
            handle: CallbackHandle::new(),
            drops: drops.clone(),
            logical_refs: Cell::new(0),
        });
        assert!(!owner.handle.begin_close());
        drop(owner);
        assert_eq!(drops.get(), 1);
    }
}
