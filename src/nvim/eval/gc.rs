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
pub fn gc_register_dict(d: &TvDict) {
    GC_STATE.with(|st| {
        let mut dicts = st.gc_dicts.borrow_mut();
        let idx = dicts.len();
        dicts.push(DictId::from(d));
        d.dv_gc_idx.set(Some(idx));
    });
}

#[inline]
pub fn gc_unregister_dict(idx: usize) {
    let Ok(moved_opt) = GC_STATE.try_with(|st| {
        let mut dicts = st.gc_dicts.borrow_mut();
        dicts.swap_remove(idx);
        dicts.get(idx).cloned()
    }) else {
        return;
    };
    if let Some(moved) = moved_opt {
        moved.dv_gc_idx.set(Some(idx));
    }
}

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static gc_first_list: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[inline]
pub fn gc_register_list(l: &TvList) {
    GC_STATE.with(|st| {
        let mut lists = st.gc_lists.borrow_mut();
        let idx = lists.len();
        lists.push(ListId::from(l));
        l.lv_gc_idx.set(Some(idx));
    });
    gc_first_list.store(
        std::ptr::from_ref(l) as usize,
        std::sync::atomic::Ordering::Relaxed,
    );
}

#[inline]
pub fn gc_unregister_list(idx: usize) {
    let Ok((moved_opt, last_opt)) = GC_STATE.try_with(|st| {
        let mut lists = st.gc_lists.borrow_mut();
        lists.swap_remove(idx);
        (lists.get(idx).cloned(), lists.last().cloned())
    }) else {
        return;
    };
    if let Some(moved) = moved_opt {
        moved.lv_gc_idx.set(Some(idx));
    }

    gc_first_list.store(
        last_opt.map_or(0, |id| id.as_ptr() as usize),
        std::sync::atomic::Ordering::Relaxed,
    );
}

#[inline]
pub fn gc_dicts_len() -> usize {
    GC_STATE.with(|st| st.gc_dicts.borrow().len())
}

#[inline]
pub fn gc_dict_at(idx: usize) -> DictId {
    GC_STATE.with(|st| st.gc_dicts.borrow()[idx].clone())
}

#[inline]
pub fn gc_free_dict_at(idx: usize) {
    let id = GC_STATE.with(|st| st.gc_dicts.borrow()[idx].clone());
    crate::eval::typval::tv_dict_free_slot(&id);
}

#[inline]
pub fn gc_lists_len() -> usize {
    GC_STATE.with(|st| st.gc_lists.borrow().len())
}

#[inline]
pub fn gc_list_at(idx: usize) -> ListId {
    GC_STATE.with(|st| st.gc_lists.borrow()[idx].clone())
}

#[inline]
pub fn gc_free_list_at(idx: usize) {
    let id = GC_STATE.with(|st| st.gc_lists.borrow()[idx].clone());
    crate::eval::typval::tv_list_free_slot(&id);
}
