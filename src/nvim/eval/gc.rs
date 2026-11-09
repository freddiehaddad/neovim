#![deny(unsafe_code)]

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
pub static gc_first_list: crate::globals::GlobalState<crate::globals::TvListPtr> =
    crate::globals::GlobalState::new(crate::globals::TvListPtr::NULL);

#[inline]
pub fn gc_register_list(l: &mut TvList) {
    let idx = gc_st().gc_lists.len();
    let ptr = crate::globals::GraphPtr::<TvList>::from_ref(l);
    gc_st().gc_lists.push(ptr);
    l.lv_gc_idx = Some(idx);
    gc_first_list.set(ptr);
}

#[inline]
pub fn gc_unregister_list(idx: usize) {
    gc_st().gc_lists.swap_remove(idx);
    if idx < gc_st().gc_lists.len() {
        let mut moved = gc_st().gc_lists.as_slice()[idx];
        moved.lv_gc_idx = Some(idx);
    }

    gc_first_list.set(
        gc_st()
            .gc_lists
            .last()
            .copied()
            .unwrap_or(crate::globals::TvListPtr::NULL),
    );
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
