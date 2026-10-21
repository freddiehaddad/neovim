//! Extended marks for plugins (`src/nvim/extmark.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::private::validate::Error;
use crate::buffer::{MAXLNUM, buf_T, kExtmarkNoUndo};
use crate::buffer_updates::buf_updates_send_splice;
use crate::cursor::{colnr_T, linenr_T, pos_T};
use crate::decoration::{
    DecorInline, buf_decor_remove, buf_put_decor, buf_signcols_count_range, decor_free,
    decor_redraw, decor_state_invalidate, decor_type_flags, kSHUIWatched, kSHUIWatchedOverlay,
};
use crate::globals::*;
use crate::kvec::KVec;
use crate::map::{
    Map, map_del_uint32_t_uint32_t, map_put_ref_uint32_t_uint32_t, map_ref_uint32_t_uint32_t,
};
use crate::marktree::{
    DECOR_HIGHLIGHT_INLINE_INIT, DecorInlineData, MT_FLAG_DECOR_HL, MT_FLAG_DECOR_SIGNTEXT,
    MT_FLAG_EXTERNAL_MASK, MT_FLAG_INVALID, MTKey, MTPair, MTPos, MarkTreeIter, marktree_clear,
    marktree_del_itr, marktree_get_alt, marktree_get_altpos, marktree_itr_current,
    marktree_itr_get, marktree_itr_get_ext, marktree_itr_get_overlap, marktree_itr_next,
    marktree_itr_step_overlap, marktree_lookup, marktree_lookup_ns, marktree_move,
    marktree_move_region, marktree_put, marktree_revise_meta, marktree_splice, mt_decor,
    mt_decor_any, mt_end, mt_flags, mt_invalid, mt_invalidate, mt_lookup_id, mt_lookup_key,
    mt_no_undo, mt_paired, mt_right, mtpair_from,
};
use crate::mcursor::{mc_ns_cleared, mc_ns_clearing, mc_on_extmark_set};
use crate::memline::ml_find_line_or_offset;
use crate::memory::{xfree, xrealloc};
use crate::undo::u_force_get_undo_header;
use std::ffi::c_int;
use std::ptr;

pub type bcount_t = isize;
pub type ExtmarkOp = c_int;
pub type ExtmarkType = c_int;
pub type ExtmarkInfoArray = KVec<MTPair>;
pub type UndoObjectType = c_int;
pub type TriState = c_int;
pub type extmark_undo_vec_t = KVec<ExtmarkUndoObject>;

pub(crate) const kExtmarkNOOP: ExtmarkOp = 0;
pub(crate) const kExtmarkUndo: ExtmarkOp = 1;
pub(crate) const kExtmarkNone: ExtmarkType = 0x1;
pub(crate) const kNone: c_int = -1;
pub(crate) const kFalse: c_int = 0;
pub(crate) const kTrue: c_int = 1;

pub(crate) const kExtmarkSplice: UndoObjectType = 0;
pub(crate) const kExtmarkMove: UndoObjectType = 1;
const kExtmarkSavePos: UndoObjectType = 3;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExtmarkSplice {
    pub start_row: c_int,
    pub start_col: colnr_T,
    pub old_row: c_int,
    pub old_col: colnr_T,
    pub new_row: c_int,
    pub new_col: colnr_T,
    pub start_byte: bcount_t,
    pub old_byte: bcount_t,
    pub new_byte: bcount_t,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExtmarkMove {
    pub start_row: c_int,
    pub start_col: c_int,
    pub extent_row: c_int,
    pub extent_col: c_int,
    pub new_row: c_int,
    pub new_col: c_int,
    pub start_byte: bcount_t,
    pub extent_byte: bcount_t,
    pub new_byte: bcount_t,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExtmarkSavePos {
    pub mark: u64,
    pub undo_row: c_int,
    pub undo_col: colnr_T,
    pub redo_row: c_int,
    pub redo_col: colnr_T,
    pub invalidated: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union ExtmarkUndoData {
    pub splice: ExtmarkSplice,
    pub move_: ExtmarkMove,
    pub savepos: ExtmarkSavePos,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExtmarkUndoObject {
    pub type_: UndoObjectType,
    pub data: ExtmarkUndoData,
}

#[inline]
unsafe fn kv_push_val<T>(v: *mut KVec<T>, val: T) {
    unsafe {
        if (*v).size == (*v).capacity {
            (*v).capacity = if (*v).capacity > 0 {
                (*v).capacity * 2
            } else {
                8
            };
            (*v).items = xrealloc((*v).items.cast(), (*v).capacity * size_of::<T>()).cast();
        }
        ptr::write((*v).items.add((*v).size), val);
        (*v).size += 1;
    }
}

#[inline]
unsafe fn map_destroy_u32_u32(map: *mut Map<u32, u32>) {
    unsafe {
        xfree((*map).set.h.hash.cast());
        xfree((*map).set.keys.cast());
        xfree((*map).values.cast());
        ptr::write_bytes(map, 0, 1);
    }
}

/// Records a raw mark's position in the open undo block, for `extmark_apply_undo()` to replay.
unsafe fn extmark_push_savepos(
    buf: *mut buf_T,
    mark: u64,
    undo_row: c_int,
    undo_col: colnr_T,
    redo_row: c_int,
    redo_col: colnr_T,
) {
    unsafe {
        let uhp = u_force_get_undo_header(buf);
        if uhp.is_null() {
            return;
        }
        kv_push_val(
            &raw mut (*uhp).uh_extmark,
            ExtmarkUndoObject {
                type_: kExtmarkSavePos,
                data: ExtmarkUndoData {
                    savepos: ExtmarkSavePos {
                        mark,
                        undo_row,
                        undo_col,
                        redo_row,
                        redo_col,
                        invalidated: false,
                    },
                },
            },
        );
    }
}

/// Create or update an extmark (`extmark_set`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_set(
    buf: *mut buf_T,
    ns_id: u32,
    idp: *mut u32,
    row: c_int,
    col: colnr_T,
    end_row: c_int,
    end_col: colnr_T,
    decor: DecorInline,
    decor_flags: u16,
    right_gravity: bool,
    end_right_gravity: bool,
    no_undo: bool,
    invalidate: bool,
    _err: *mut Error,
) {
    unsafe {
        let ns = map_put_ref_uint32_t_uint32_t(
            (*buf).b_extmark_ns.as_mut_ptr(),
            ns_id,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        let mut id = if !idp.is_null() { *idp } else { 0 };
        let mut created = true;

        let flags = mt_flags(right_gravity, no_undo, invalidate, decor.ext) | decor_flags;
        let mut revised_in_place = false;
        if id == 0 {
            *ns += 1;
            id = *ns;
        } else {
            let mut itr = MarkTreeIter::default();
            let old_mark = marktree_lookup_ns(
                (*buf).b_marktree.as_mut_ptr(),
                ns_id,
                id,
                false,
                &raw mut itr,
            );
            if old_mark.id != 0 {
                created = false;
                if !no_undo
                    && !(*buf).b_u_synced
                    && (old_mark.pos.row != row || old_mark.pos.col != col)
                {
                    extmark_push_savepos(
                        buf,
                        mt_lookup_key(old_mark),
                        old_mark.pos.row,
                        old_mark.pos.col,
                        row,
                        col,
                    );
                }
                if mt_paired(old_mark) || end_row > -1 {
                    extmark_del_id(buf, ns_id, id);
                } else {
                    debug_assert!(!itr.x.is_null());
                    if old_mark.pos.row == row && old_mark.pos.col == col {
                        if !mt_invalid(old_mark) && mt_decor_any(old_mark) {
                            (*itr.x).key[itr.i as usize].flags &= !MT_FLAG_EXTERNAL_MASK;
                            buf_decor_remove(buf, row, row, col, mt_decor(old_mark), true);
                        }
                        (*itr.x).key[itr.i as usize].flags |= flags;
                        (*itr.x).key[itr.i as usize].decor_data = decor.data;
                        marktree_revise_meta(
                            (*buf).b_marktree.as_mut_ptr(),
                            &raw mut itr,
                            old_mark,
                        );
                        revised_in_place = true;
                    } else {
                        marktree_del_itr((*buf).b_marktree.as_mut_ptr(), &raw mut itr, false);
                        if !mt_invalid(old_mark) {
                            buf_decor_remove(
                                buf,
                                old_mark.pos.row,
                                old_mark.pos.row,
                                old_mark.pos.col,
                                mt_decor(old_mark),
                                true,
                            );
                        }
                    }
                }
            } else {
                *ns = (*ns).max(id);
            }
        }

        if !revised_in_place {
            let mark = MTKey {
                pos: MTPos { row, col },
                ns: ns_id,
                id,
                flags,
                decor_data: decor.data,
            };

            marktree_put(
                (*buf).b_marktree.as_mut_ptr(),
                mark,
                end_row,
                end_col,
                end_right_gravity,
            );
            decor_state_invalidate(buf);

            if created && !no_undo && !(*buf).b_u_synced {
                extmark_push_savepos(buf, mt_lookup_id(ns_id, id, false), -1, 0, row, col);
                if end_row > -1 {
                    extmark_push_savepos(
                        buf,
                        mt_lookup_id(ns_id, id, true),
                        -1,
                        0,
                        end_row,
                        end_col,
                    );
                }
            }
        }

        if decor_flags != 0 || decor.ext {
            let r2 = if end_row > -1 { end_row } else { row };
            buf_put_decor(buf, decor, row, r2);
            decor_redraw(buf, row, r2, col, decor);
        }

        if !idp.is_null() {
            *idp = id;
        }

        if created {
            mc_on_extmark_set(
                buf,
                ns_id,
                id,
                pos_T {
                    lnum: row + 1,
                    col,
                    coladd: 0,
                },
            );
        }
    }
}

/// Moves a raw mark back to a recorded position (`extmark_setraw`).
unsafe fn extmark_setraw(buf: *mut buf_T, mark: u64, row: c_int, col: colnr_T, revive: bool) {
    unsafe {
        let mut itr = MarkTreeIter::default();
        let key = marktree_lookup((*buf).b_marktree.as_mut_ptr(), mark, &raw mut itr);
        let move_ = key.pos.row != row || key.pos.col != col;
        let invalid = revive && mt_invalid(key);
        if key.pos.row < 0 || (!move_ && !invalid) {
            return;
        }

        if !invalid && mt_decor_any(key) && key.pos.row != row {
            decor_redraw(buf, key.pos.row, key.pos.row, key.pos.col, mt_decor(key));
        }

        let mut row1: c_int = 0;
        let mut row2: c_int = 0;
        let mut altitr = itr;
        let alt = marktree_get_alt((*buf).b_marktree.as_mut_ptr(), key, &raw mut altitr);

        if invalid {
            (*itr.x).key[itr.i as usize].flags &= !MT_FLAG_INVALID;
            (*altitr.x).key[altitr.i as usize].flags &= !MT_FLAG_INVALID;
            marktree_revise_meta(
                (*buf).b_marktree.as_mut_ptr(),
                if mt_end(key) {
                    &raw mut altitr
                } else {
                    &raw mut itr
                },
                if mt_end(key) { alt } else { key },
            );
        } else if !mt_invalid(key)
            && (key.flags & MT_FLAG_DECOR_SIGNTEXT) != 0
            && (*buf).b_signcols.autom
        {
            row1 = alt.pos.row.min(key.pos.row.min(row));
            row2 = alt.pos.row.max(key.pos.row.max(row));
            buf_signcols_count_range(
                buf,
                row1,
                ((*curbuf).b_ml.ml_line_count - 1).min(row2),
                0,
                kTrue,
            );
        }

        if move_ {
            marktree_move((*buf).b_marktree.as_mut_ptr(), &raw mut itr, row, col);
        }

        if invalid {
            buf_put_decor(
                buf,
                mt_decor(key),
                row.min(alt.pos.row),
                row.max(alt.pos.row),
            );
        } else if !mt_invalid(key)
            && (key.flags & MT_FLAG_DECOR_SIGNTEXT) != 0
            && (*buf).b_signcols.autom
        {
            buf_signcols_count_range(
                buf,
                row1,
                ((*curbuf).b_ml.ml_line_count - 1).min(row2),
                0,
                kNone,
            );
        }
    }
}

/// Remove an extmark in `ns_id` by `id` (`extmark_del_id`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_del_id(buf: *mut buf_T, ns_id: u32, id: u32) -> bool {
    unsafe {
        let mut itr = MarkTreeIter::default();
        let key = marktree_lookup_ns(
            (*buf).b_marktree.as_mut_ptr(),
            ns_id,
            id,
            false,
            &raw mut itr,
        );
        if key.id != 0 {
            extmark_del(buf, &raw mut itr, key, false);
        }
        key.id > 0
    }
}

/// Remove a (paired) extmark `key` pointed to by `itr` (`extmark_del`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_del(
    buf: *mut buf_T,
    itr: *mut MarkTreeIter,
    mut key: MTKey,
    restore: bool,
) {
    unsafe {
        debug_assert!(key.pos.row >= 0);

        let mut key2 = key;
        let other = marktree_del_itr((*buf).b_marktree.as_mut_ptr(), itr, false);
        if other != 0 {
            key2 = marktree_lookup((*buf).b_marktree.as_mut_ptr(), other, itr);
            debug_assert!(key2.pos.row >= 0);
            marktree_del_itr((*buf).b_marktree.as_mut_ptr(), itr, false);
            if restore {
                marktree_itr_get(
                    (*buf).b_marktree.as_mut_ptr(),
                    key.pos.row,
                    key.pos.col,
                    itr,
                );
            }
        }

        if mt_decor_any(key) {
            if mt_invalid(key) {
                decor_free(mt_decor(key));
            } else {
                if mt_end(key) {
                    std::mem::swap(&mut key, &mut key2);
                }
                buf_decor_remove(
                    buf,
                    key.pos.row,
                    key2.pos.row,
                    key.pos.col,
                    mt_decor(key),
                    true,
                );
            }
        }

        decor_state_invalidate(buf);
    }
}

/// Free extmarks in a ns between lines (`extmark_clear`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_clear(
    buf: *mut buf_T,
    ns_id: u32,
    l_row: c_int,
    l_col: colnr_T,
    u_row: c_int,
    u_col: colnr_T,
) -> bool {
    unsafe {
        if (*buf).b_extmark_ns[0].set.h.n_keys == 0 {
            return false;
        }

        mc_ns_clearing(buf, ns_id);

        let all_ns = ns_id == 0;
        if !all_ns {
            let ns =
                map_ref_uint32_t_uint32_t((*buf).b_extmark_ns.as_mut_ptr(), ns_id, ptr::null_mut());
            if ns.is_null() {
                return false;
            }
        }

        let mut marks_cleared_any = false;
        let mut marks_cleared_all = l_row == 0 && l_col == 0;

        let mut itr = MarkTreeIter::default();
        marktree_itr_get((*buf).b_marktree.as_mut_ptr(), l_row, l_col, &raw mut itr);
        loop {
            let mark = marktree_itr_current(&raw mut itr);
            if mark.pos.row < 0
                || mark.pos.row > u_row
                || (mark.pos.row == u_row && mark.pos.col > u_col)
            {
                if mark.pos.row >= 0 {
                    marks_cleared_all = false;
                }
                break;
            }
            if mark.ns == ns_id || all_ns {
                marks_cleared_any = true;
                extmark_del(buf, &raw mut itr, mark, true);
            } else {
                marktree_itr_next((*buf).b_marktree.as_mut_ptr(), &raw mut itr);
            }
        }

        if marks_cleared_all {
            if all_ns {
                map_destroy_u32_u32((*buf).b_extmark_ns.as_mut_ptr());
            } else {
                map_del_uint32_t_uint32_t((*buf).b_extmark_ns.as_mut_ptr(), ns_id, ptr::null_mut());
            }
        }

        if marks_cleared_any {
            decor_state_invalidate(buf);
            mc_ns_cleared(buf, ns_id);
        }

        marks_cleared_any
    }
}

/// Return the position of marks between a range (`extmark_get`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_get(
    buf: *mut buf_T,
    ns_id: u32,
    l_row: c_int,
    l_col: colnr_T,
    u_row: c_int,
    u_col: colnr_T,
    amount: i64,
    type_filter: ExtmarkType,
    overlap: bool,
) -> ExtmarkInfoArray {
    unsafe {
        let mut array = ExtmarkInfoArray {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
        let mut itr = MarkTreeIter::default();

        if overlap {
            if !marktree_itr_get_overlap((*buf).b_marktree.as_mut_ptr(), l_row, l_col, &raw mut itr)
            {
                return array;
            }

            while (array.size as i64) < amount {
                let mut pair = std::mem::zeroed();
                if !marktree_itr_step_overlap(
                    (*buf).b_marktree.as_mut_ptr(),
                    &raw mut itr,
                    &raw mut pair,
                ) {
                    break;
                }
                push_mark(&raw mut array, ns_id, type_filter, pair);
            }
        } else {
            marktree_itr_get_ext(
                (*buf).b_marktree.as_mut_ptr(),
                MTPos {
                    row: l_row,
                    col: l_col,
                },
                &raw mut itr,
                false,
                false,
                ptr::null_mut(),
                ptr::null(),
            );
        }

        while (array.size as i64) < amount {
            let mark = marktree_itr_current(&raw mut itr);
            if mark.pos.row < 0
                || mark.pos.row > u_row
                || (mark.pos.row == u_row && mark.pos.col > u_col)
            {
                break;
            }
            if !mt_end(mark) {
                let end = marktree_get_alt((*buf).b_marktree.as_mut_ptr(), mark, ptr::null_mut());
                push_mark(&raw mut array, ns_id, type_filter, mtpair_from(mark, end));
            }
            marktree_itr_next((*buf).b_marktree.as_mut_ptr(), &raw mut itr);
        }
        array
    }
}

unsafe fn push_mark(
    array: *mut ExtmarkInfoArray,
    ns_id: u32,
    type_filter: ExtmarkType,
    mark: MTPair,
) {
    unsafe {
        if !(ns_id == u32::MAX || mark.start.ns == ns_id) {
            return;
        }
        if type_filter != kExtmarkNone {
            if !mt_decor_any(mark.start) {
                return;
            }
            let type_flags = decor_type_flags(mt_decor(mark.start));
            if (type_flags as ExtmarkType & type_filter) == 0 {
                return;
            }
        }

        kv_push_val(array, mark);
    }
}

/// Lookup an extmark by id (`extmark_from_id`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_from_id(buf: *mut buf_T, ns_id: u32, id: u32) -> MTPair {
    unsafe {
        let mark = marktree_lookup_ns(
            (*buf).b_marktree.as_mut_ptr(),
            ns_id,
            id,
            false,
            ptr::null_mut(),
        );
        if mark.id == 0 {
            return mtpair_from(mark, mark);
        }
        debug_assert!(mark.pos.row >= 0);
        let end = marktree_get_alt((*buf).b_marktree.as_mut_ptr(), mark, ptr::null_mut());

        mtpair_from(mark, end)
    }
}

/// Creates or updates point extmark (`extmark_set_pos`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_set_pos(
    buf: *mut buf_T,
    ns_id: u32,
    mark: *mut u32,
    pos: pos_T,
    right_gravity: bool,
    no_undo: bool,
    ui_watched: bool,
) {
    unsafe {
        let mut decor = DecorInline {
            ext: false,
            data: DecorInlineData {
                hl: DECOR_HIGHLIGHT_INLINE_INIT,
            },
        };
        if ui_watched {
            decor.data.hl.flags = kSHUIWatched | kSHUIWatchedOverlay;
        }
        extmark_set(
            buf,
            ns_id,
            mark,
            pos.lnum - 1,
            pos.col,
            -1,
            0,
            decor,
            if ui_watched { MT_FLAG_DECOR_HL } else { 0 },
            right_gravity,
            false,
            no_undo,
            false,
            ptr::null_mut(),
        );
    }
}

/// Gets the position of an extmark, shifted by buffer edits since it was set (`extmark_get_pos`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_get_pos(
    buf: *mut buf_T,
    ns_id: u32,
    id: u32,
    pos: *mut pos_T,
) -> bool {
    unsafe {
        let mtp = extmark_from_id(buf, ns_id, id);
        if mtp.start.id == 0 {
            return false;
        }
        (*pos).lnum = mtp.start.pos.row + 1;
        (*pos).col = mtp.start.pos.col;
        true
    }
}

/// Free extmarks from the buffer (`extmark_free_all`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_free_all(buf: *mut buf_T) {
    unsafe {
        let mut itr = MarkTreeIter::default();
        marktree_itr_get((*buf).b_marktree.as_mut_ptr(), 0, 0, &raw mut itr);
        loop {
            let mark = marktree_itr_current(&raw mut itr);
            if mark.pos.row < 0 {
                break;
            }

            if !(mt_paired(mark) && mt_end(mark)) {
                decor_free(mt_decor(mark));
            }

            marktree_itr_next((*buf).b_marktree.as_mut_ptr(), &raw mut itr);
        }

        marktree_clear((*buf).b_marktree.as_mut_ptr());

        (*buf).b_signcols.max = 0;
        (*buf).b_signcols.count = [0; 9];

        map_destroy_u32_u32((*buf).b_extmark_ns.as_mut_ptr());
    }
}

/// Invalidate extmarks between range and copy to undo header (`extmark_splice_delete`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_splice_delete(
    buf: *mut buf_T,
    l_row: c_int,
    l_col: colnr_T,
    u_row: c_int,
    u_col: colnr_T,
    uvp: *mut extmark_undo_vec_t,
    only_copy: bool,
    op: ExtmarkOp,
) {
    unsafe {
        let mut itr = MarkTreeIter::default();
        marktree_itr_get((*buf).b_marktree.as_mut_ptr(), l_row, l_col, &raw mut itr);
        loop {
            let mark = marktree_itr_current(&raw mut itr);
            if mark.pos.row < 0 || mark.pos.row > u_row {
                break;
            }

            let mut copy = true;
            if mark.pos.row == l_row && mark.pos.col - c_int::from(!mt_right(mark)) < l_col {
                copy = false;
            } else if mark.pos.row == u_row {
                if mark.pos.col > u_col + 1 {
                    break;
                } else if mark.pos.col + c_int::from(mt_right(mark)) > u_col {
                    copy = false;
                }
            }

            let mut invalidated = false;
            if !only_copy && !mt_invalid(mark) && mt_invalidate(mark) && !mt_end(mark) {
                let mut enditr = itr;
                let endpos =
                    marktree_get_altpos((*buf).b_marktree.as_mut_ptr(), mark, &raw mut enditr);
                if (!mt_paired(mark) && mark.pos.row < u_row)
                    || (mt_paired(mark)
                        && (mark.pos.row > l_row
                            || (mark.pos.row == l_row && mark.pos.col >= l_col))
                        && (endpos.row < u_row || (endpos.row == u_row && endpos.col <= u_col)))
                {
                    if mt_no_undo(mark) {
                        extmark_del(buf, &raw mut itr, mark, true);
                        continue;
                    } else {
                        copy = true;
                        invalidated = true;
                        (*itr.x).key[itr.i as usize].flags |= MT_FLAG_INVALID;
                        (*enditr.x).key[enditr.i as usize].flags |= MT_FLAG_INVALID;
                        marktree_revise_meta((*buf).b_marktree.as_mut_ptr(), &raw mut itr, mark);
                        buf_decor_remove(
                            buf,
                            mark.pos.row,
                            endpos.row,
                            mark.pos.col,
                            mt_decor(mark),
                            false,
                        );
                    }
                }
            }

            if copy && (only_copy || (!uvp.is_null() && op == kExtmarkUndo && !mt_no_undo(mark))) {
                let pos = ExtmarkSavePos {
                    mark: mt_lookup_key(mark),
                    invalidated,
                    undo_row: mark.pos.row,
                    undo_col: mark.pos.col,
                    redo_row: -1,
                    redo_col: -1,
                };
                kv_push_val(
                    uvp,
                    ExtmarkUndoObject {
                        type_: kExtmarkSavePos,
                        data: ExtmarkUndoData { savepos: pos },
                    },
                );
            }

            marktree_itr_next((*buf).b_marktree.as_mut_ptr(), &raw mut itr);
        }
    }
}

/// Undo or redo an extmark operation (`extmark_apply_undo`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_apply_undo(undo_info: ExtmarkUndoObject, undo: bool) {
    unsafe {
        if undo_info.type_ == kExtmarkSplice {
            let splice = undo_info.data.splice;
            if undo {
                extmark_splice_impl(
                    curbuf,
                    splice.start_row,
                    splice.start_col,
                    splice.start_byte,
                    splice.new_row,
                    splice.new_col,
                    splice.new_byte,
                    splice.old_row,
                    splice.old_col,
                    splice.old_byte,
                    kExtmarkNoUndo,
                );
            } else {
                extmark_splice_impl(
                    curbuf,
                    splice.start_row,
                    splice.start_col,
                    splice.start_byte,
                    splice.old_row,
                    splice.old_col,
                    splice.old_byte,
                    splice.new_row,
                    splice.new_col,
                    splice.new_byte,
                    kExtmarkNoUndo,
                );
            }
        } else if undo_info.type_ == kExtmarkSavePos {
            let pos = undo_info.data.savepos;
            if undo && pos.undo_row >= 0 {
                extmark_setraw(
                    curbuf,
                    pos.mark,
                    pos.undo_row,
                    pos.undo_col,
                    pos.invalidated,
                );
            } else if !undo && pos.redo_row >= 0 {
                extmark_setraw(curbuf, pos.mark, pos.redo_row, pos.redo_col, true);
            }
        } else if undo_info.type_ == kExtmarkMove {
            let move_ = undo_info.data.move_;
            if undo {
                extmark_move_region(
                    curbuf,
                    move_.new_row,
                    move_.new_col,
                    move_.new_byte,
                    move_.extent_row,
                    move_.extent_col,
                    move_.extent_byte,
                    move_.start_row,
                    move_.start_col,
                    move_.start_byte,
                    kExtmarkNoUndo,
                );
            } else {
                extmark_move_region(
                    curbuf,
                    move_.start_row,
                    move_.start_col,
                    move_.start_byte,
                    move_.extent_row,
                    move_.extent_col,
                    move_.extent_byte,
                    move_.new_row,
                    move_.new_col,
                    move_.new_byte,
                    kExtmarkNoUndo,
                );
            }
        }
    }
}

/// Adjust extmark row for inserted/deleted rows (`extmark_adjust`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_adjust(
    buf: *mut buf_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
    undo: ExtmarkOp,
) {
    unsafe {
        if curbuf_splice_pending != 0 {
            return;
        }
        let start_byte = ml_find_line_or_offset(buf, line1, ptr::null_mut(), true) as bcount_t;
        let mut old_byte: bcount_t = 0;
        let mut new_byte: bcount_t = 0;
        let old_row: c_int;
        let new_row: c_int;
        if amount == MAXLNUM {
            old_row = line2 - line1 + 1;
            old_byte = (*buf).deleted_bytes2 as bcount_t;
            new_row = amount_after + old_row;
        } else {
            debug_assert_eq!(line2, MAXLNUM);
            old_row = 0;
            new_row = amount as c_int;
        }
        if new_row > 0 {
            new_byte = (ml_find_line_or_offset(buf, line1 + new_row, ptr::null_mut(), true)
                as bcount_t)
                - start_byte;
        }
        extmark_splice_impl(
            buf,
            line1 - 1,
            0,
            start_byte,
            old_row,
            0,
            old_byte,
            new_row,
            0,
            new_byte,
            undo,
        );
    }
}

/// Adjusts extmarks after a text edit, and emits the `on_bytes` event (`extmark_splice`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_splice(
    buf: *mut buf_T,
    start_row: c_int,
    start_col: colnr_T,
    old_row: c_int,
    old_col: colnr_T,
    old_byte: bcount_t,
    new_row: c_int,
    new_col: colnr_T,
    new_byte: bcount_t,
    undo: ExtmarkOp,
) {
    unsafe {
        let mut offset = ml_find_line_or_offset(buf, start_row + 1, ptr::null_mut(), true);
        if offset < 0 && (*buf).b_ml.ml_chunksize.is_null() {
            offset = 0;
        }
        extmark_splice_impl(
            buf,
            start_row,
            start_col,
            (offset + start_col) as bcount_t,
            old_row,
            old_col,
            old_byte,
            new_row,
            new_col,
            new_byte,
            undo,
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_splice_impl(
    buf: *mut buf_T,
    start_row: c_int,
    start_col: colnr_T,
    start_byte: bcount_t,
    old_row: c_int,
    old_col: colnr_T,
    old_byte: bcount_t,
    new_row: c_int,
    new_col: colnr_T,
    new_byte: bcount_t,
    undo: ExtmarkOp,
) {
    unsafe {
        (*buf).deleted_bytes2 = 0;
        buf_updates_send_splice(
            buf, start_row, start_col, start_byte, old_row, old_col, old_byte, new_row, new_col,
            new_byte,
        );

        if old_row > 0 || old_col > 0 {
            let end_row = start_row + old_row;
            let end_col = (if old_row != 0 { 0 } else { start_col }) + old_col;
            let uhp = u_force_get_undo_header(buf);
            let uvp = if !uhp.is_null() {
                &raw mut (*uhp).uh_extmark
            } else {
                ptr::null_mut()
            };
            extmark_splice_delete(
                buf, start_row, start_col, end_row, end_col, uvp, false, undo,
            );
        }

        if old_row > 0 || new_row > 0 {
            let count = if (*buf).b_prev_line_count > 0 {
                (*buf).b_prev_line_count
            } else {
                (*buf).b_ml.ml_line_count
            };
            buf_signcols_count_range(
                buf,
                start_row,
                (count - 1).min(start_row + old_row),
                0,
                kTrue,
            );
            (*buf).b_prev_line_count = 0;
        }

        marktree_splice(
            (*buf).b_marktree.as_mut_ptr(),
            start_row,
            start_col,
            old_row,
            old_col,
            new_row,
            new_col,
        );

        if old_row > 0 || new_row > 0 {
            let row2 = ((*buf).b_ml.ml_line_count - 1).min(start_row + new_row);
            buf_signcols_count_range(buf, start_row, row2, 0, kNone);
        }

        if undo == kExtmarkUndo {
            let uhp = u_force_get_undo_header(buf);
            if uhp.is_null() {
                return;
            }

            let mut merged = false;
            if old_row == 0 && new_row == 0 && (*uhp).uh_extmark.size > 0 {
                let item = (*uhp).uh_extmark.items.add((*uhp).uh_extmark.size - 1);
                if (*item).type_ == kExtmarkSplice {
                    let splice = &raw mut (*item).data.splice;
                    if (*splice).start_row == start_row
                        && (*splice).old_row == 0
                        && (*splice).new_row == 0
                    {
                        if old_col == 0
                            && start_col >= (*splice).start_col
                            && start_col <= (*splice).start_col + (*splice).new_col
                        {
                            (*splice).new_col += new_col;
                            (*splice).new_byte += new_byte;
                            merged = true;
                        } else if new_col == 0
                            && start_col == (*splice).start_col + (*splice).new_col
                        {
                            (*splice).old_col += old_col;
                            (*splice).old_byte += old_byte;
                            merged = true;
                        } else if new_col == 0 && start_col + old_col == (*splice).start_col {
                            (*splice).start_col = start_col;
                            (*splice).start_byte = start_byte;
                            (*splice).old_col += old_col;
                            (*splice).old_byte += old_byte;
                            merged = true;
                        }
                    }
                }
            }

            if !merged {
                let splice = ExtmarkSplice {
                    start_row,
                    start_col,
                    old_row,
                    old_col,
                    new_row,
                    new_col,
                    start_byte,
                    old_byte,
                    new_byte,
                };
                kv_push_val(
                    &raw mut (*uhp).uh_extmark,
                    ExtmarkUndoObject {
                        type_: kExtmarkSplice,
                        data: ExtmarkUndoData { splice },
                    },
                );
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_splice_cols(
    buf: *mut buf_T,
    start_row: c_int,
    start_col: colnr_T,
    old_col: colnr_T,
    new_col: colnr_T,
    undo: ExtmarkOp,
) {
    unsafe {
        extmark_splice(
            buf,
            start_row,
            start_col,
            0,
            old_col,
            old_col as bcount_t,
            0,
            new_col,
            new_col as bcount_t,
            undo,
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn extmark_move_region(
    buf: *mut buf_T,
    start_row: c_int,
    start_col: colnr_T,
    start_byte: bcount_t,
    extent_row: c_int,
    extent_col: colnr_T,
    extent_byte: bcount_t,
    new_row: c_int,
    new_col: colnr_T,
    new_byte: bcount_t,
    undo: ExtmarkOp,
) {
    unsafe {
        (*buf).deleted_bytes2 = 0;
        buf_updates_send_splice(
            buf,
            start_row,
            start_col,
            start_byte,
            extent_row,
            extent_col,
            extent_byte,
            0,
            0,
            0,
        );

        let row1 = start_row.min(new_row);
        let row2 = start_row.max(new_row) + extent_row;
        buf_signcols_count_range(buf, row1, row2, 0, kTrue);

        marktree_move_region(
            (*buf).b_marktree.as_mut_ptr(),
            start_row,
            start_col,
            extent_row,
            extent_col,
            new_row,
            new_col,
        );

        buf_signcols_count_range(buf, row1, row2, 0, kNone);

        buf_updates_send_splice(
            buf,
            new_row,
            new_col,
            new_byte,
            0,
            0,
            0,
            extent_row,
            extent_col,
            extent_byte,
        );

        if undo == kExtmarkUndo {
            let uhp = u_force_get_undo_header(buf);
            if uhp.is_null() {
                return;
            }

            let move_ = ExtmarkMove {
                start_row,
                start_col,
                extent_row,
                extent_col,
                new_row,
                new_col,
                start_byte,
                extent_byte,
                new_byte,
            };
            kv_push_val(
                &raw mut (*uhp).uh_extmark,
                ExtmarkUndoObject {
                    type_: kExtmarkMove,
                    data: ExtmarkUndoData { move_ },
                },
            );
        }
    }
}
