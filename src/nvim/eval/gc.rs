//! Garbage collection linked-list roots for dicts and lists (`src/nvim/eval/gc.c`).

use crate::eval::typval::{dict_T, list_T};

/// Head of list of all dictionaries (`gc_first_dict`).
pub static mut gc_first_dict: *mut dict_T = std::ptr::null_mut();

/// Head of list of all lists (`gc_first_list`).
#[unsafe(no_mangle)]
pub static mut gc_first_list: *mut list_T = std::ptr::null_mut();
