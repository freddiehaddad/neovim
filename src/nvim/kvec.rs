//! C-ABI compatible `kvec_t` and `kvec_withinit_t` containers (`src/klib/kvec.h`).

use std::ffi::c_void;

use crate::memory::{xfree, xmalloc, xrealloc};

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

impl<T: Copy> KVec<T> {
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

    /// Push `value` onto `self` (`kv_push`).
    ///
    /// # Safety
    /// `self.items` must be null or allocated by `xmalloc`/`xrealloc`.
    #[inline]
    pub unsafe fn push(&mut self, value: T) {
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
        unsafe {
            *self.items.add(self.size) = value;
        }
        self.size += 1;
    }

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

impl<T: Copy> Default for KVec<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct KVecWithInit<T, const N: usize> {
    pub size: usize,
    pub capacity: usize,
    pub items: *mut T,
    pub init_array: [T; N],
}

impl<T: Copy, const N: usize> KVecWithInit<T, N> {
    #[inline]
    pub fn init(&mut self) {
        self.size = 0;
        self.capacity = N;
        self.items = self.init_array.as_mut_ptr();
    }

    /// Read element at `index`.
    ///
    /// # Safety
    /// `index` must be `< self.size` and `self.items` must be valid.
    #[inline]
    pub unsafe fn get(&self, index: usize) -> T {
        unsafe { *self.items.add(index) }
    }

    /// Read the last element.
    ///
    /// # Safety
    /// `self.size` must be `> 0` and `self.items` must be valid.
    #[inline]
    pub unsafe fn last(&self) -> T {
        assert!(self.size > 0);
        unsafe { *self.items.add(self.size - 1) }
    }

    /// Resize the backing storage to at least `new_cap`.
    ///
    /// # Safety
    /// `self` must be an initialized `KVecWithInit`.
    pub unsafe fn resize(&mut self, new_cap: usize) {
        let cap = new_cap.max(N);
        let init_ptr = self.init_array.as_mut_ptr();
        let elem_size = std::mem::size_of::<T>();

        if cap == N {
            if !std::ptr::eq(self.items, init_ptr) {
                let old_items = self.items;
                unsafe {
                    std::ptr::copy_nonoverlapping(old_items, init_ptr, self.size);
                    xfree(old_items.cast::<c_void>());
                }
                self.items = init_ptr;
            }
        } else if std::ptr::eq(self.items, init_ptr) {
            let new_items = unsafe { xmalloc(cap * elem_size).cast::<T>() };
            unsafe {
                std::ptr::copy_nonoverlapping(init_ptr, new_items, self.size);
            }
            self.items = new_items;
        } else {
            self.items =
                unsafe { xrealloc(self.items.cast::<c_void>(), cap * elem_size).cast::<T>() };
        }
        self.capacity = cap;
    }

    /// Push `value` onto this vector.
    ///
    /// # Safety
    /// `self` must be an initialized `KVecWithInit`.
    pub unsafe fn push(&mut self, value: T) {
        if self.size == self.capacity {
            let next_cap = self.capacity << 1;
            unsafe {
                self.resize(next_cap);
            }
        }
        unsafe {
            *self.items.add(self.size) = value;
        }
        self.size += 1;
    }

    /// Free heap storage if `items` moved off `init_array`.
    ///
    /// # Safety
    /// `self` must be an initialized `KVecWithInit`.
    pub unsafe fn destroy(&mut self) {
        if !std::ptr::eq(self.items, self.init_array.as_ptr()) {
            unsafe {
                xfree(self.items.cast::<c_void>());
            }
            self.items = std::ptr::null_mut();
        }
    }
}
