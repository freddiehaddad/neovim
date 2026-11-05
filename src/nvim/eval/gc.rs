//! Garbage collection registries for dicts and lists (`src/nvim/eval/gc.c`).

use crate::eval::typval::{TvDict, TvList};

struct GcModuleState {
    gc_dicts: Vec<crate::globals::GraphPtr<TvDict>>,
    gc_lists: Vec<crate::globals::GraphPtr<TvList>>,
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
    GC_STATE.as_mut()
}

#[inline]
pub fn gc_register_dict(d: &mut TvDict) {
    let idx = gc_st().gc_dicts.len();
    gc_st()
        .gc_dicts
        .push(crate::globals::GraphPtr::from(&mut *d));
    d.dv_gc_idx = Some(idx);
}

#[inline]
pub fn gc_unregister_dict(idx: usize) {
    gc_st().gc_dicts.swap_remove(idx);
    if idx < gc_st().gc_dicts.len() {
        let mut moved = gc_st().gc_dicts.as_slice()[idx];
        moved.dv_gc_idx = Some(idx);
    }
}

#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static gc_first_list: crate::globals::GlobalState<*mut TvList> =
    crate::globals::GlobalState::new(std::ptr::null_mut());

#[inline]
pub fn gc_register_list(l: &mut TvList) {
    let idx = gc_st().gc_lists.len();
    let ptr: *mut TvList = l;
    gc_st().gc_lists.push(crate::globals::GraphPtr::new(ptr));
    l.lv_gc_idx = Some(idx);
    *gc_first_list.as_mut() = ptr;
}

#[inline]
pub fn gc_unregister_list(idx: usize) {
    gc_st().gc_lists.swap_remove(idx);
    if idx < gc_st().gc_lists.len() {
        let mut moved = gc_st().gc_lists.as_slice()[idx];
        moved.lv_gc_idx = Some(idx);
    }

    *gc_first_list.as_mut() = gc_st()
        .gc_lists
        .last()
        .map_or(std::ptr::null_mut(), |&p| p.as_ptr());
}

#[inline]
pub fn gc_dicts_len() -> usize {
    gc_st().gc_dicts.len()
}

#[inline]
pub fn gc_dict_at(idx: usize) -> &'static mut TvDict {
    gc_st().gc_dicts[idx].borrow_mut()
}

#[inline]
pub fn gc_free_dict_at(idx: usize) {
    drop(unsafe { Box::from_raw(gc_st().gc_dicts[idx].as_ptr()) });
}

#[inline]
pub fn gc_lists_len() -> usize {
    gc_st().gc_lists.len()
}

#[inline]
pub fn gc_list_at(idx: usize) -> &'static mut TvList {
    gc_st().gc_lists[idx].borrow_mut()
}

#[inline]
pub fn gc_free_list_at(idx: usize) {
    drop(unsafe { Box::from_raw(gc_st().gc_lists[idx].as_ptr()) });
}
