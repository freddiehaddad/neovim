//! Garbage collection registries for dicts and lists (`src/nvim/eval/gc.c`).

use crate::eval::typval::{TvDict, TvList};

struct GcModuleState {
    gc_dicts: Vec<*mut TvDict>,
    gc_lists: Vec<*mut TvList>,
}

impl GcModuleState {
    const fn new() -> Self {
        Self {
            gc_dicts: Vec::new(),
            gc_lists: Vec::new(),
        }
    }
}

static GC_STATE: crate::globals::GlobalState<GcModuleState> =
    crate::globals::GlobalState::new(GcModuleState::new());

#[inline]
fn gc_st() -> &'static mut GcModuleState {
    unsafe { &mut *GC_STATE.as_ptr() }
}

#[inline]
pub fn gc_register_dict(d: &mut TvDict) {
    let idx = gc_st().gc_dicts.len();
    gc_st().gc_dicts.push(d);
    d.dv_gc_idx = Some(idx);
}

#[inline]
pub fn gc_unregister_dict(idx: usize) {
    gc_st().gc_dicts.swap_remove(idx);
    if idx < gc_st().gc_dicts.len() {
        let moved = gc_st().gc_dicts.as_slice()[idx];
        unsafe {
            (*moved).dv_gc_idx = Some(idx);
        }
    }
}

#[inline]
pub fn gc_register_list(l: &mut TvList) {
    let idx = gc_st().gc_lists.len();
    gc_st().gc_lists.push(l);
    l.lv_gc_idx = Some(idx);
}

#[inline]
pub fn gc_unregister_list(idx: usize) {
    gc_st().gc_lists.swap_remove(idx);
    if idx < gc_st().gc_lists.len() {
        let moved = gc_st().gc_lists.as_slice()[idx];
        unsafe {
            (*moved).lv_gc_idx = Some(idx);
        }
    }
}

#[inline]
pub fn gc_dicts_slice() -> &'static [*mut TvDict] {
    gc_st().gc_dicts.as_slice()
}

#[inline]
pub fn gc_lists_slice() -> &'static [*mut TvList] {
    gc_st().gc_lists.as_slice()
}
