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

    pub fn is_uninitialized(&self) -> bool {
        self.phase.get() == Phase::Uninitialized
    }

    pub fn is_active(&self) -> bool {
        self.phase.get() == Phase::Active
    }

    pub fn is_closed(&self) -> bool {
        self.phase.get() == Phase::Closed
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

struct CallbackState<T: 'static, const WORDS: usize> {
    native: CallbackHandle<Self, WORDS>,
    data: T,
}

pub struct SharedCallbackHandle<T: 'static, const WORDS: usize>(Rc<CallbackState<T, WORDS>>);

impl<T: 'static, const WORDS: usize> Clone for SharedCallbackHandle<T, WORDS> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static, const WORDS: usize> SharedCallbackHandle<T, WORDS> {
    pub fn new(data: T) -> Self {
        Self(Rc::new(CallbackState {
            native: CallbackHandle::new(),
            data,
        }))
    }

    pub fn data(&self) -> &T {
        &self.0.data
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.0.native.as_ptr()
    }

    pub fn can_initialize(&self) -> bool {
        self.0.native.is_uninitialized()
    }

    pub fn is_active(&self) -> bool {
        self.0.native.is_active()
    }

    pub fn is_closed(&self) -> bool {
        self.0.native.is_closed()
    }

    /// # Safety
    /// The native handle must be initialized in this storage. Its callbacks
    /// must recover this registration type on the owning thread.
    pub unsafe fn register(&self) {
        assert!(
            self.can_initialize(),
            "native storage cannot be reinitialized"
        );
        unsafe { self.0.native.register(self.0.clone()) };
    }

    /// # Safety
    /// `handle` must carry this type's live native registration, and the caller
    /// must run on its owning thread.
    pub unsafe fn from_callback(handle: *mut c_void) -> Self {
        Self(unsafe { CallbackHandle::<CallbackState<T, WORDS>, WORDS>::callback_owner(handle) })
    }

    pub fn begin_close(&self) -> bool {
        self.0.native.begin_close()
    }

    /// # Safety
    /// Call only after native use has finished, from the native close callback.
    /// This handle retains the state while the registration is released.
    pub unsafe fn finish_close(&self) {
        drop(unsafe { self.0.native.finish_close() });
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

    #[test]
    fn native_bytes_remain_mutable_while_shared_metadata_is_borrowed() {
        fn native_write(handle: &SharedCallbackHandle<usize, 80>, pointer: *mut c_void) {
            let metadata = handle.data();
            unsafe {
                pointer
                    .byte_add(size_of::<*const c_void>())
                    .cast::<u8>()
                    .write(2);
            }
            assert_eq!(*metadata, 7);
            let callback = unsafe { SharedCallbackHandle::<usize, 80>::from_callback(pointer) };
            assert_eq!(*callback.data(), 7);
        }

        let handle = SharedCallbackHandle::new(7);
        let pointer = handle.as_ptr();
        unsafe { handle.register() };
        native_write(&handle, pointer);
        assert!(handle.begin_close());
        unsafe { handle.finish_close() };
    }
}
