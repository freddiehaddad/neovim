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

    /// Ensure at least `len` additional items fit in `self` (`kv_ensure_space`).
    ///
    /// # Safety
    /// `self.items` must be null or allocated by `xmalloc`/`xrealloc`.
    #[inline]
    pub unsafe fn ensure_space(&mut self, len: usize) {
        if self.capacity < self.size + len {
            let mut new_cap = self.size + len;
            new_cap = new_cap.next_power_of_two().max(1);
            self.capacity = new_cap;
            self.items = unsafe {
                xrealloc(
                    self.items.cast::<c_void>(),
                    std::mem::size_of::<T>() * self.capacity,
                )
                .cast::<T>()
            };
        }
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

impl<T: Copy> KVec<T> {
    /// Append `len` elements from `data` (`kv_concat_len`).
    ///
    /// # Safety
    /// `data` must be valid for `len` elements when `len > 0`.
    #[inline]
    pub unsafe fn concat_len(&mut self, data: *const T, len: usize) {
        if len > 0 {
            unsafe {
                self.ensure_space(len);
                std::ptr::copy_nonoverlapping(data, self.items.add(self.size), len);
            }
            self.size += len;
        }
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
    fn kvec_push_concat_and_destroy() {
        unsafe {
            let mut v: KVec<i32> = KVec::new();
            for i in 0..20 {
                v.push(i);
            }
            v.concat_len([100, 200].as_ptr(), 2);
            assert_eq!(v.size, 22);
            let slice = std::slice::from_raw_parts(v.items, v.size);
            assert_eq!(slice[0], 0);
            assert_eq!(slice[19], 19);
            assert_eq!(slice[20], 100);
            assert_eq!(slice[21], 200);
            v.destroy();
            assert_eq!(v.size, 0);
            assert!(v.items.is_null());
        }
    }
}
