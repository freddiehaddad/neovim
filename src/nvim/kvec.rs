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
    pub fn init(&mut self) {
        self.size = 0;
        self.capacity = 0;
        self.items = std::ptr::null_mut();
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
