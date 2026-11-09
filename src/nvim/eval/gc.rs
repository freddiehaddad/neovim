#![deny(unsafe_code)]

//! Garbage collection registries for dicts and lists (`src/nvim/eval/gc.c`).

use crate::eval::typval::{ListId, TvDict, TvList};

struct GcModuleState {
    gc_dicts: Vec<crate::globals::GraphPtr<TvDict>>,
    gc_lists: Vec<ListId>,
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
fn gc_st() -> crate::globals::GraphPtr<GcModuleState> {
    GC_STATE.handle()
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

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static gc_first_list: crate::globals::GlobalState<usize> = crate::globals::GlobalState::new(0);

#[inline]
pub fn gc_register_list(l: &mut TvList) {
    let idx = gc_st().gc_lists.len();
    gc_st().gc_lists.push(l.lv_id);
    l.lv_gc_idx = Some(idx);
    gc_first_list.set(std::ptr::from_mut(l) as usize);
}

#[inline]
pub fn gc_unregister_list(idx: usize) {
    gc_st().gc_lists.swap_remove(idx);
    if idx < gc_st().gc_lists.len() {
        let mut moved = gc_st().gc_lists.as_slice()[idx];
        moved.lv_gc_idx = Some(idx);
    }

    gc_first_list.set(gc_st().gc_lists.last().map_or(0, |id| id.as_ptr() as usize));
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
    crate::eval::typval::tv_dict_free_slot(gc_st().gc_dicts[idx].borrow_mut());
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
    crate::eval::typval::tv_list_free_slot(gc_st().gc_lists[idx].borrow_mut());
}
