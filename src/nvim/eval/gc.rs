//! Garbage collection registries for dicts and lists (`src/nvim/eval/gc.c`).

use crate::eval::typval::{TvDict, TvList};

struct GcModuleState {
    GC_DICTS: Vec<*mut TvDict>,
    GC_LISTS: Vec<*mut TvList>,
}

impl GcModuleState {
    const fn new() -> Self {
        Self {
            GC_DICTS: Vec::new(),
            GC_LISTS: Vec::new(),
        }
    }
}

static GC_STATE: crate::globals::GlobalState<GcModuleState> = crate::globals::GlobalState::new(GcModuleState::new());

#[inline]
fn gc_st() -> &'static mut GcModuleState {
    unsafe { &mut *GC_STATE.as_ptr() }
}

#[inline]
pub fn gc_register_dict(d: &mut TvDict) {
     {
        let idx = gc_st().GC_DICTS.len();
        gc_st().GC_DICTS.push(d);
        d.dv_gc_idx = Some(idx);
    }
}

#[inline]
pub fn gc_unregister_dict(idx: usize) {
    unsafe {
        gc_st().GC_DICTS.swap_remove(idx);
        if idx < gc_st().GC_DICTS.len() {
            let moved = gc_st().GC_DICTS.as_slice()[idx];
            (*moved).dv_gc_idx = Some(idx);
        }
    }
}

#[inline]
pub fn gc_register_list(l: &mut TvList) {
     {
        let idx = gc_st().GC_LISTS.len();
        gc_st().GC_LISTS.push(l);
        l.lv_gc_idx = Some(idx);
    }
}

#[inline]
pub fn gc_unregister_list(idx: usize) {
    unsafe {
        gc_st().GC_LISTS.swap_remove(idx);
        if idx < gc_st().GC_LISTS.len() {
            let moved = gc_st().GC_LISTS.as_slice()[idx];
            (*moved).lv_gc_idx = Some(idx);
        }
    }
}

#[inline]
pub fn gc_dicts_slice() -> &'static [*mut TvDict] {
     {
        gc_st().GC_DICTS.as_slice()
    }
}

#[inline]
pub fn gc_lists_slice() -> &'static [*mut TvList] {
     {
        gc_st().GC_LISTS.as_slice()
    }
}

