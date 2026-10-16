//! Garbage collection linked-list roots for dicts and lists (`src/nvim/eval/gc.c`).

use std::ffi::c_void;

/// Head of list of all dictionaries (`gc_first_dict`).
#[unsafe(no_mangle)]
pub static mut gc_first_dict: *mut c_void = std::ptr::null_mut();

/// Head of list of all lists (`gc_first_list`).
#[unsafe(no_mangle)]
pub static mut gc_first_list: *mut c_void = std::ptr::null_mut();
