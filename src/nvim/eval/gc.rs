//! Garbage collection registries for dicts and lists (`src/nvim/eval/gc.c`).

use crate::eval::typval::{dict_T, list_T};

static mut GC_DICTS: Vec<*mut dict_T> = Vec::new();
static mut GC_LISTS: Vec<*mut list_T> = Vec::new();

#[inline]
pub unsafe fn gc_register_dict(d: *mut dict_T) {
    unsafe {
        let dicts = &raw mut GC_DICTS;
        let idx = (*dicts).len();
        (*dicts).push(d);
        (*d).dv_gc_idx = Some(idx);
    }
}

#[inline]
pub unsafe fn gc_unregister_dict(idx: usize) {
    unsafe {
        let dicts = &raw mut GC_DICTS;
        (*dicts).swap_remove(idx);
        if idx < (*dicts).len() {
            let moved = (*dicts).as_slice()[idx];
            (*moved).dv_gc_idx = Some(idx);
        }
    }
}

#[inline]
pub unsafe fn gc_register_list(l: *mut list_T) {
    unsafe {
        let lists = &raw mut GC_LISTS;
        let idx = (*lists).len();
        (*lists).push(l);
        (*l).lv_gc_idx = Some(idx);
    }
}

#[inline]
pub unsafe fn gc_unregister_list(idx: usize) {
    unsafe {
        let lists = &raw mut GC_LISTS;
        (*lists).swap_remove(idx);
        if idx < (*lists).len() {
            let moved = (*lists).as_slice()[idx];
            (*moved).lv_gc_idx = Some(idx);
        }
    }
}

#[inline]
pub unsafe fn gc_dicts_slice() -> &'static [*mut dict_T] {
    unsafe {
        let ptr = &raw const GC_DICTS;
        (*ptr).as_slice()
    }
}

#[inline]
pub unsafe fn gc_lists_slice() -> &'static [*mut list_T] {
    unsafe {
        let ptr = &raw const GC_LISTS;
        (*ptr).as_slice()
    }
}

