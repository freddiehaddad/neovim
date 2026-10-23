//! C-ABI compatible `kvec_t` container (`src/klib/kvec.h`).

use std::ffi::c_void;

use crate::memory::{xfree, xrealloc};

#[repr(C)]
#[derive(Debug)]
pub struct KVec<T> {
    pub size: usize,
    pub capacity: usize,
    pub items: *mut T,
}

impl<T> Clone for KVec<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for KVec<T> {}

impl<T> KVec<T> {
    #[inline]
    pub const fn new() -> Self {
        Self {
            size: 0,
            capacity: 0,
            items: std::ptr::null_mut(),
        }
    }

    #[inline]
    pub fn init(&mut self) {
        self.size = 0;
        self.capacity = 0;
        self.items = std::ptr::null_mut();
    }

    /// Reserve one slot at the end of `self` and return a pointer to it (`kv_pushp`).
    ///
    /// # Safety
    /// `self.items` must be null or allocated by `xmalloc`/`xrealloc`.
    #[inline]
    pub unsafe fn pushp(&mut self) -> *mut T {
        if self.size == self.capacity {
            let new_cap = if self.capacity != 0 {
                self.capacity << 1
            } else {
                8
            };
            self.capacity = new_cap;
            self.items = unsafe {
                xrealloc(
                    self.items.cast::<c_void>(),
                    std::mem::size_of::<T>() * self.capacity,
                )
                .cast::<T>()
            };
        }
        let ptr = unsafe { self.items.add(self.size) };
        self.size += 1;
        ptr
    }

    /// Push `value` onto `self` (`kv_push`).
    ///
    /// # Safety
    /// `self.items` must be null or allocated by `xmalloc`/`xrealloc`.
    #[inline]
    pub unsafe fn push(&mut self, value: T) {
        unsafe {
            std::ptr::write(self.pushp(), value);
        }
    }

    /// Free the backing buffer of this `KVec` and reset its state.
    ///
    /// # Safety
    /// `self.items` must be null or allocated by `xmalloc`/`xrealloc`.
    #[inline]
    pub unsafe fn destroy(&mut self) {
        unsafe {
            xfree(self.items.cast::<c_void>());
        }
        self.init();
    }
}

impl<T> Default for KVec<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kvec_push_and_destroy() {
        unsafe {
            let mut v: KVec<i32> = KVec::new();
            for i in 0..20 {
                v.push(i);
            }
            assert_eq!(v.size, 20);
            let slice = std::slice::from_raw_parts(v.items, v.size);
            assert_eq!(slice[0], 0);
            assert_eq!(slice[19], 19);
            v.destroy();
            assert_eq!(v.size, 0);
            assert!(v.items.is_null());
        }
    }
}
