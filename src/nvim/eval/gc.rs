#![deny(unsafe_code)]

//! Garbage collection registries for dicts and lists (`src/nvim/eval/gc.c`).

use crate::eval::typval::{DictId, ListId, TvDict, TvList};

struct GcModuleState {
    gc_dicts: std::cell::RefCell<Vec<DictId>>,
    gc_lists: std::cell::RefCell<Vec<ListId>>,
}

impl GcModuleState {
    const fn new() -> Self {
        Self {
            gc_dicts: std::cell::RefCell::new(Vec::new()),
            gc_lists: std::cell::RefCell::new(Vec::new()),
        }
    }
}

std::thread_local! {
    static GC_STATE: GcModuleState = const { GcModuleState::new() };
}

#[inline]
pub fn gc_register_dict(d: &mut TvDict) {
    GC_STATE.with(|st| {
        let mut dicts = st.gc_dicts.borrow_mut();
        let idx = dicts.len();
        dicts.push(d.dv_id);
        d.dv_gc_idx = Some(idx);
    });
}

#[inline]
pub fn gc_unregister_dict(idx: usize) {
    let moved_opt = GC_STATE.with(|st| {
        let mut dicts = st.gc_dicts.borrow_mut();
        dicts.swap_remove(idx);
        dicts.get(idx).copied()
    });
    if let Some(mut moved) = moved_opt {
        moved.dv_gc_idx = Some(idx);
    }
}

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static gc_first_list: crate::globals::GlobalState<usize> = crate::globals::GlobalState::new(0);

#[inline]
pub fn gc_register_list(l: &mut TvList) {
    GC_STATE.with(|st| {
        let mut lists = st.gc_lists.borrow_mut();
        let idx = lists.len();
        lists.push(l.lv_id);
        l.lv_gc_idx = Some(idx);
    });
    gc_first_list.set(std::ptr::from_mut(l) as usize);
}

#[inline]
pub fn gc_unregister_list(idx: usize) {
    let (moved_opt, last_opt) = GC_STATE.with(|st| {
        let mut lists = st.gc_lists.borrow_mut();
        lists.swap_remove(idx);
        (lists.get(idx).copied(), lists.last().copied())
    });
    if let Some(mut moved) = moved_opt {
        moved.lv_gc_idx = Some(idx);
    }

    gc_first_list.set(last_opt.map_or(0, |id| id.as_ptr() as usize));
}

#[inline]
pub fn gc_dicts_len() -> usize {
    GC_STATE.with(|st| st.gc_dicts.borrow().len())
}

#[inline]
pub fn gc_dict_at(idx: usize) -> &'static mut TvDict {
    let id = GC_STATE.with(|st| st.gc_dicts.borrow()[idx]);
    id.borrow_mut()
}

#[inline]
pub fn gc_free_dict_at(idx: usize) {
    let id = GC_STATE.with(|st| st.gc_dicts.borrow()[idx]);
    crate::eval::typval::tv_dict_free_slot(id.borrow_mut());
}

#[inline]
pub fn gc_lists_len() -> usize {
    GC_STATE.with(|st| st.gc_lists.borrow().len())
}

#[inline]
pub fn gc_list_at(idx: usize) -> &'static mut TvList {
    let id = GC_STATE.with(|st| st.gc_lists.borrow()[idx]);
    id.borrow_mut()
}

#[inline]
pub fn gc_free_list_at(idx: usize) {
    let id = GC_STATE.with(|st| st.gc_lists.borrow()[idx]);
    crate::eval::typval::tv_list_free_slot(id.borrow_mut());
}
