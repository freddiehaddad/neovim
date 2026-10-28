//! Extended marks for plugins (`src/nvim/extmark.c`).

use crate::buffer::{FileBuf, K_EXTMARK_NO_UNDO, MAXLNUM};
use crate::buffer_updates::buf_updates_send_splice;
use crate::cursor::{Colnr, CursorPos, Linenr};
use crate::decoration::{
    DecorInline, K_SHUIWATCHED, K_SHUIWATCHED_OVERLAY, buf_decor_remove, buf_put_decor,
    buf_signcols_count_range, decor_free, decor_redraw, decor_state_invalidate, decor_type_flags,
};
use crate::globals::*;
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
use crate::undo::u_force_get_undo_header;
use std::ffi::c_int;
use std::ptr;

pub type Bcount = isize;
pub type ExtmarkOp = c_int;
pub type ExtmarkType = u16;
pub type ExtmarkInfoArray = Vec<MTPair>;
pub type UndoObjectType = c_int;
pub type TriState = c_int;
pub type ExtmarkUndoVec = Vec<ExtmarkUndoObject>;

pub(crate) const K_EXTMARK_NOOP: ExtmarkOp = 0;
pub(crate) const K_EXTMARK_UNDO: ExtmarkOp = 1;
pub(crate) const K_EXTMARK_NONE: ExtmarkType = 0x1;
pub(crate) const K_EXTMARK_SIGN: ExtmarkType = 0x2;
pub(crate) const K_EXTMARK_VIRT_TEXT: ExtmarkType = 0x8;
pub(crate) const K_EXTMARK_VIRT_LINES: ExtmarkType = 0x10;
pub(crate) const K_EXTMARK_HIGHLIGHT: ExtmarkType = 0x20;
pub(crate) const K_NONE: c_int = -1;
pub(crate) const K_FALSE: c_int = 0;
pub(crate) const K_TRUE: c_int = 1;

pub(crate) const K_EXTMARK_SPLICE: UndoObjectType = 0;
pub(crate) const K_EXTMARK_MOVE: UndoObjectType = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ExtmarkSplice {
    pub start_row: c_int,
    pub start_col: Colnr,
    pub old_row: c_int,
    pub old_col: Colnr,
    pub new_row: c_int,
    pub new_col: Colnr,
    pub start_byte: Bcount,
    pub old_byte: Bcount,
    pub new_byte: Bcount,
}

impl ExtmarkSplice {
    pub const fn new() -> Self {
        Self {
            start_row: 0,
            start_col: 0,
            old_row: 0,
            old_col: 0,
            new_row: 0,
            new_col: 0,
            start_byte: 0,
            old_byte: 0,
            new_byte: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ExtmarkMove {
    pub start_row: c_int,
    pub start_col: c_int,
    pub extent_row: c_int,
    pub extent_col: c_int,
    pub new_row: c_int,
    pub new_col: c_int,
    pub start_byte: Bcount,
    pub extent_byte: Bcount,
    pub new_byte: Bcount,
}

impl ExtmarkMove {
    pub const fn new() -> Self {
        Self {
            start_row: 0,
            start_col: 0,
            extent_row: 0,
            extent_col: 0,
            new_row: 0,
            new_col: 0,
            start_byte: 0,
            extent_byte: 0,
            new_byte: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExtmarkSavePos {
    pub mark: u64,
    pub undo_row: c_int,
    pub undo_col: Colnr,
    pub redo_row: c_int,
    pub redo_col: Colnr,
    pub invalidated: bool,
}

#[derive(Clone, Copy)]
pub enum ExtmarkUndoObject {
    Splice(ExtmarkSplice),
    Move(ExtmarkMove),
    SavePos(ExtmarkSavePos),
}

/// Records a raw mark's position in the open undo block, for `extmark_apply_undo()` to replay.
fn extmark_push_savepos(
    buf: &mut FileBuf,
    mark: u64,
    undo_row: c_int,
    undo_col: Colnr,
    redo_row: c_int,
    redo_col: Colnr,
) {
    {
        let Some(uh) = u_force_get_undo_header(&mut *buf) else {
            return;
        };
        crate::undo::uhp_mut(&mut *buf, uh)
            .uh_extmark
            .push(ExtmarkUndoObject::SavePos(ExtmarkSavePos {
                mark,
                undo_row,
                undo_col,
                redo_row,
                redo_col,
                invalidated: false,
            }));
    }
}

/// Create or update an extmark (`extmark_set`).
pub fn extmark_set(
    buf: &mut FileBuf,
    ns_id: u32,
    mut idp: Option<&mut u32>,
    row: c_int,
    col: Colnr,
    end_row: c_int,
    end_col: Colnr,
    decor: DecorInline,
    decor_flags: u16,
    right_gravity: bool,
    end_right_gravity: bool,
    no_undo: bool,
    invalidate: bool,
) {
    {
        let ns = buf.b_extmark_ns.entry(ns_id).or_insert(0);
        let mut id = if idp.is_some() {
            *idp.as_deref_mut().unwrap()
        } else {
            0
        };
        let mut created = true;

        let flags = mt_flags(right_gravity, no_undo, invalidate, decor.is_ext()) | decor_flags;
        let mut revised_in_place = false;
        if id == 0 {
            *ns += 1;
            id = *ns;
        } else {
            let mut itr = MarkTreeIter::new();
            let old_mark = marktree_lookup_ns(&buf.b_marktree, ns_id, id, false, Some(&mut itr));
            if old_mark.id != 0 {
                created = false;
                if !no_undo
                    && !buf.b_u_synced
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
                    debug_assert!(itr.x.is_some());
                    if old_mark.pos.row == row && old_mark.pos.col == col {
                        if !mt_invalid(old_mark) && mt_decor_any(old_mark) {
                            buf.b_marktree.key_mut(&itr).flags &= !MT_FLAG_EXTERNAL_MASK;
                            buf_decor_remove(buf, row, row, col, mt_decor(old_mark), true);
                        }
                        buf.b_marktree.key_mut(&itr).flags |= flags;
                        buf.b_marktree.key_mut(&itr).decor_data = decor.data;
                        marktree_revise_meta(&mut buf.b_marktree, &itr, old_mark);
                        revised_in_place = true;
                    } else {
                        marktree_del_itr(&mut buf.b_marktree, &mut itr, false);
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
                &mut buf.b_marktree,
                mark,
                end_row,
                end_col,
                end_right_gravity,
            );
            decor_state_invalidate(buf);

            if created && !no_undo && !buf.b_u_synced {
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

        if decor_flags != 0 || decor.is_ext() {
            let r2 = if end_row > -1 { end_row } else { row };
            buf_put_decor(&mut *buf, decor, row, r2);
            decor_redraw(&mut *buf, row, r2, col, decor);
        }

        if let Some(ref mut p) = idp {
            **p = id;
        }

        if created {
            mc_on_extmark_set(
                buf,
                ns_id,
                id,
                CursorPos {
                    lnum: row + 1,
                    col,
                    coladd: 0,
                },
            );
        }
    }
}

/// Moves a raw mark back to a recorded position (`extmark_setraw`).
fn extmark_setraw(buf: &mut FileBuf, mark: u64, row: c_int, col: Colnr, revive: bool) {
    {
        let mut itr = MarkTreeIter::new();
        let key = marktree_lookup(&buf.b_marktree, mark, Some(&mut itr));
        let move_ = key.pos.row != row || key.pos.col != col;
        let invalid = revive && mt_invalid(key);
        if key.pos.row < 0 || (!move_ && !invalid) {
            return;
        }

        if !invalid && mt_decor_any(key) && key.pos.row != row {
            decor_redraw(
                &mut *buf,
                key.pos.row,
                key.pos.row,
                key.pos.col,
                mt_decor(key),
            );
        }

        let mut row1: c_int = 0;
        let mut row2: c_int = 0;
        let mut altitr = itr;
        let alt = marktree_get_alt(&buf.b_marktree, key, Some(&mut altitr));

        if invalid {
            buf.b_marktree.key_mut(&itr).flags &= !MT_FLAG_INVALID;
            buf.b_marktree.key_mut(&altitr).flags &= !MT_FLAG_INVALID;
            marktree_revise_meta(
                &mut buf.b_marktree,
                if mt_end(key) { &altitr } else { &itr },
                if mt_end(key) { alt } else { key },
            );
        } else if !mt_invalid(key)
            && (key.flags & MT_FLAG_DECOR_SIGNTEXT) != 0
            && buf.b_signcols.autom
        {
            row1 = alt.pos.row.min(key.pos.row.min(row));
            row2 = alt.pos.row.max(key.pos.row.max(row));
            buf_signcols_count_range(
                &mut *buf,
                row1,
                (curbuf_mut().b_ml.ml_line_count - 1).min(row2),
                0,
                K_TRUE,
            );
        }

        if move_ {
            marktree_move(&mut buf.b_marktree, &mut itr, row, col);
        }

        if invalid {
            buf_put_decor(
                &mut *buf,
                mt_decor(key),
                row.min(alt.pos.row),
                row.max(alt.pos.row),
            );
        } else if !mt_invalid(key)
            && (key.flags & MT_FLAG_DECOR_SIGNTEXT) != 0
            && buf.b_signcols.autom
        {
            buf_signcols_count_range(
                &mut *buf,
                row1,
                (curbuf_mut().b_ml.ml_line_count - 1).min(row2),
                0,
                K_NONE,
            );
        }
    }
}

/// Remove an extmark in `ns_id` by `id` (`extmark_del_id`).
pub fn extmark_del_id(buf: &mut FileBuf, ns_id: u32, id: u32) -> bool {
    let mut itr = MarkTreeIter::new();
    let key = marktree_lookup_ns(&buf.b_marktree, ns_id, id, false, Some(&mut itr));
    if key.id != 0 {
        extmark_del(buf, &mut itr, key, false);
    }
    key.id > 0
}

/// Remove a (paired) extmark `key` pointed to by `itr` (`extmark_del`).
pub fn extmark_del(buf: &mut FileBuf, itr: &mut MarkTreeIter, mut key: MTKey, restore: bool) {
    debug_assert!(key.pos.row >= 0);

    let mut key2 = key;
    let other = marktree_del_itr(&mut buf.b_marktree, itr, false);
    if other != 0 {
        key2 = marktree_lookup(&buf.b_marktree, other, Some(itr));
        debug_assert!(key2.pos.row >= 0);
        marktree_del_itr(&mut buf.b_marktree, itr, false);
        if restore {
            marktree_itr_get(&buf.b_marktree, key.pos.row, key.pos.col, itr);
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

/// Free extmarks in a ns between lines (`extmark_clear`).
pub fn extmark_clear(
    buf: &mut FileBuf,
    ns_id: u32,
    l_row: c_int,
    l_col: Colnr,
    u_row: c_int,
    u_col: Colnr,
) -> bool {
    {
        if buf.b_extmark_ns.is_empty() {
            return false;
        }

        mc_ns_clearing(buf, ns_id);

        let all_ns = ns_id == 0;
        if !all_ns && !buf.b_extmark_ns.contains_key(&ns_id) {
            return false;
        }

        let mut marks_cleared_any = false;
        let mut marks_cleared_all = l_row == 0 && l_col == 0;

        let mut itr = MarkTreeIter::new();
        marktree_itr_get(&buf.b_marktree, l_row, l_col, &mut itr);
        loop {
            let mark = marktree_itr_current(&buf.b_marktree, &itr);
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
                extmark_del(buf, &mut itr, mark, true);
            } else {
                marktree_itr_next(&buf.b_marktree, &mut itr);
            }
        }

        if marks_cleared_all {
            if all_ns {
                buf.b_extmark_ns.clear();
            } else {
                buf.b_extmark_ns.remove(&ns_id);
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
pub fn extmark_get(
    buf: &mut FileBuf,
    ns_id: u32,
    l_row: c_int,
    l_col: Colnr,
    u_row: c_int,
    u_col: Colnr,
    amount: i64,
    type_filter: ExtmarkType,
    overlap: bool,
) -> ExtmarkInfoArray {
    let mut array = ExtmarkInfoArray::new();
    let mut itr = MarkTreeIter::new();

    if overlap {
        if !marktree_itr_get_overlap(&buf.b_marktree, l_row, l_col, &mut itr) {
            return array;
        }

        while (array.len() as i64) < amount {
            let mut pair = MTPair::new();
            if !marktree_itr_step_overlap(&buf.b_marktree, &mut itr, &mut pair) {
                break;
            }
            push_mark(&mut array, ns_id, type_filter, pair);
        }
    } else {
        marktree_itr_get_ext(
            &buf.b_marktree,
            MTPos {
                row: l_row,
                col: l_col,
            },
            &mut itr,
            false,
            false,
            None,
            None,
        );
    }

    while (array.len() as i64) < amount {
        let mark = marktree_itr_current(&buf.b_marktree, &itr);
        if mark.pos.row < 0
            || mark.pos.row > u_row
            || (mark.pos.row == u_row && mark.pos.col > u_col)
        {
            break;
        }
        if !mt_end(mark) {
            let end = marktree_get_alt(&buf.b_marktree, mark, None);
            push_mark(&mut array, ns_id, type_filter, mtpair_from(mark, end));
        }
        marktree_itr_next(&buf.b_marktree, &mut itr);
    }
    array
}

fn push_mark(array: &mut ExtmarkInfoArray, ns_id: u32, type_filter: ExtmarkType, mark: MTPair) {
    if !(ns_id == u32::MAX || mark.start.ns == ns_id) {
        return;
    }
    if type_filter != K_EXTMARK_NONE {
        if !mt_decor_any(mark.start) {
            return;
        }
        let type_flags = decor_type_flags(mt_decor(mark.start));
        if (type_flags & type_filter) == 0 {
            return;
        }
    }

    array.push(mark);
}

/// Lookup an extmark by id (`extmark_from_id`).
pub fn extmark_from_id(buf: &mut FileBuf, ns_id: u32, id: u32) -> MTPair {
    let mark = marktree_lookup_ns(&buf.b_marktree, ns_id, id, false, None);
    if mark.id == 0 {
        return mtpair_from(mark, mark);
    }
    debug_assert!(mark.pos.row >= 0);
    let end = marktree_get_alt(&buf.b_marktree, mark, None);

    mtpair_from(mark, end)
}

/// Creates or updates point extmark (`extmark_set_pos`).
pub fn extmark_set_pos(
    buf: &mut FileBuf,
    ns_id: u32,
    mark: &mut u32,
    pos: CursorPos,
    right_gravity: bool,
    no_undo: bool,
    ui_watched: bool,
) {
    {
        let mut decor = DecorInline {
            data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
        };
        if ui_watched {
            decor.data.hl_mut().flags = K_SHUIWATCHED | K_SHUIWATCHED_OVERLAY;
        }
        extmark_set(
            buf,
            ns_id,
            Some(&mut *mark),
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
        );
    }
}

/// Gets the position of an extmark, shifted by buffer edits since it was set (`extmark_get_pos`).
pub fn extmark_get_pos(buf: &mut FileBuf, ns_id: u32, id: u32, pos: &mut CursorPos) -> bool {
    {
        let mtp = extmark_from_id(buf, ns_id, id);
        if mtp.start.id == 0 {
            return false;
        }
        pos.lnum = mtp.start.pos.row + 1;
        pos.col = mtp.start.pos.col;
        true
    }
}

/// Free extmarks from the buffer (`extmark_free_all`).
pub fn extmark_free_all(buf: &mut FileBuf) {
    let mut itr = MarkTreeIter::new();
    marktree_itr_get(&buf.b_marktree, 0, 0, &mut itr);
    loop {
        let mark = marktree_itr_current(&buf.b_marktree, &itr);
        if mark.pos.row < 0 {
            break;
        }

        if !(mt_paired(mark) && mt_end(mark)) {
            decor_free(mt_decor(mark));
        }

        marktree_itr_next(&buf.b_marktree, &mut itr);
    }

    marktree_clear(&mut buf.b_marktree);

    buf.b_signcols.max = 0;
    buf.b_signcols.count = [0; 9];

    buf.b_extmark_ns.clear();
}

/// Invalidate extmarks between range and copy to undo header (`extmark_splice_delete`).
pub fn extmark_splice_delete(
    buf: &mut FileBuf,
    l_row: c_int,
    l_col: Colnr,
    u_row: c_int,
    u_col: Colnr,
    mut uvp: Option<&mut ExtmarkUndoVec>,
    only_copy: bool,
    op: ExtmarkOp,
) {
    {
        let mut itr = MarkTreeIter::new();
        marktree_itr_get(&buf.b_marktree, l_row, l_col, &mut itr);
        loop {
            let mark = marktree_itr_current(&buf.b_marktree, &itr);
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
                let endpos = marktree_get_altpos(&buf.b_marktree, mark, Some(&mut enditr));
                if (!mt_paired(mark) && mark.pos.row < u_row)
                    || (mt_paired(mark)
                        && (mark.pos.row > l_row
                            || (mark.pos.row == l_row && mark.pos.col >= l_col))
                        && (endpos.row < u_row || (endpos.row == u_row && endpos.col <= u_col)))
                {
                    if mt_no_undo(mark) {
                        extmark_del(buf, &mut itr, mark, true);
                        continue;
                    } else {
                        copy = true;
                        invalidated = true;
                        buf.b_marktree.key_mut(&itr).flags |= MT_FLAG_INVALID;
                        buf.b_marktree.key_mut(&enditr).flags |= MT_FLAG_INVALID;
                        marktree_revise_meta(&mut buf.b_marktree, &itr, mark);
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

            if copy && (only_copy || (uvp.is_some() && op == K_EXTMARK_UNDO && !mt_no_undo(mark))) {
                let pos = ExtmarkSavePos {
                    mark: mt_lookup_key(mark),
                    invalidated,
                    undo_row: mark.pos.row,
                    undo_col: mark.pos.col,
                    redo_row: -1,
                    redo_col: -1,
                };
                uvp.as_deref_mut()
                    .unwrap()
                    .push(ExtmarkUndoObject::SavePos(pos));
            }

            marktree_itr_next(&buf.b_marktree, &mut itr);
        }
    }
}

/// Undo or redo an extmark operation (`extmark_apply_undo`).
pub fn extmark_apply_undo(undo_info: ExtmarkUndoObject, undo: bool) {
    {
        match undo_info {
            ExtmarkUndoObject::Splice(splice) => {
                if undo {
                    extmark_splice_impl(
                        curbuf_mut(),
                        splice.start_row,
                        splice.start_col,
                        splice.start_byte,
                        splice.new_row,
                        splice.new_col,
                        splice.new_byte,
                        splice.old_row,
                        splice.old_col,
                        splice.old_byte,
                        K_EXTMARK_NO_UNDO,
                    );
                } else {
                    extmark_splice_impl(
                        curbuf_mut(),
                        splice.start_row,
                        splice.start_col,
                        splice.start_byte,
                        splice.old_row,
                        splice.old_col,
                        splice.old_byte,
                        splice.new_row,
                        splice.new_col,
                        splice.new_byte,
                        K_EXTMARK_NO_UNDO,
                    );
                }
            }
            ExtmarkUndoObject::SavePos(pos) => {
                if undo && pos.undo_row >= 0 {
                    extmark_setraw(
                        curbuf_mut(),
                        pos.mark,
                        pos.undo_row,
                        pos.undo_col,
                        pos.invalidated,
                    );
                } else if !undo && pos.redo_row >= 0 {
                    extmark_setraw(curbuf_mut(), pos.mark, pos.redo_row, pos.redo_col, true);
                }
            }
            ExtmarkUndoObject::Move(move_) => {
                if undo {
                    extmark_move_region(
                        curbuf_mut(),
                        move_.new_row,
                        move_.new_col,
                        move_.new_byte,
                        move_.extent_row,
                        move_.extent_col,
                        move_.extent_byte,
                        move_.start_row,
                        move_.start_col,
                        move_.start_byte,
                        K_EXTMARK_NO_UNDO,
                    );
                } else {
                    extmark_move_region(
                        curbuf_mut(),
                        move_.start_row,
                        move_.start_col,
                        move_.start_byte,
                        move_.extent_row,
                        move_.extent_col,
                        move_.extent_byte,
                        move_.new_row,
                        move_.new_col,
                        move_.new_byte,
                        K_EXTMARK_NO_UNDO,
                    );
                }
            }
        }
    }
}

/// Adjust extmark row for inserted/deleted rows (`extmark_adjust`).
pub fn extmark_adjust(
    buf: &mut FileBuf,
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
    undo: ExtmarkOp,
) {
    {
        if g_st().curbuf_splice_pending != 0 {
            return;
        }
        let start_byte = ml_find_line_or_offset(buf, line1, None, true) as Bcount;
        let mut old_byte: Bcount = 0;
        let mut new_byte: Bcount = 0;
        let old_row: c_int;
        let new_row: c_int;
        if amount == MAXLNUM {
            old_row = line2 - line1 + 1;
            old_byte = buf.deleted_bytes2 as Bcount;
            new_row = amount_after + old_row;
        } else {
            debug_assert_eq!(line2, MAXLNUM);
            old_row = 0;
            new_row = amount as c_int;
        }
        if new_row > 0 {
            new_byte =
                (ml_find_line_or_offset(buf, line1 + new_row, None, true) as Bcount) - start_byte;
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
pub fn extmark_splice(
    buf: &mut FileBuf,
    start_row: c_int,
    start_col: Colnr,
    old_row: c_int,
    old_col: Colnr,
    old_byte: Bcount,
    new_row: c_int,
    new_col: Colnr,
    new_byte: Bcount,
    undo: ExtmarkOp,
) {
    {
        let mut offset = ml_find_line_or_offset(buf, start_row + 1, None, true);
        if offset < 0 && buf.b_ml.ml_chunksize.is_empty() {
            offset = 0;
        }
        extmark_splice_impl(
            buf,
            start_row,
            start_col,
            (offset + start_col) as Bcount,
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

pub fn extmark_splice_impl(
    buf: &mut FileBuf,
    start_row: c_int,
    start_col: Colnr,
    start_byte: Bcount,
    old_row: c_int,
    old_col: Colnr,
    old_byte: Bcount,
    new_row: c_int,
    new_col: Colnr,
    new_byte: Bcount,
    undo: ExtmarkOp,
) {
    buf.deleted_bytes2 = 0;
    buf_updates_send_splice(
        &mut *buf, start_row, start_col, start_byte, old_row, old_col, old_byte, new_row, new_col,
        new_byte,
    );

    if old_row > 0 || old_col > 0 {
        let end_row = start_row + old_row;
        let end_col = (if old_row != 0 { 0 } else { start_col }) + old_col;
        let uhp = u_force_get_undo_header(buf);
        let uvp = if let Some(uh) = uhp {
            &raw mut crate::undo::uhp_mut(buf, uh).uh_extmark
        } else {
            ptr::null_mut()
        };
        extmark_splice_delete(
            buf,
            start_row,
            start_col,
            end_row,
            end_col,
            unsafe { uvp.as_mut() },
            false,
            undo,
        );
    }

    if old_row > 0 || new_row > 0 {
        let count = if buf.b_prev_line_count > 0 {
            buf.b_prev_line_count
        } else {
            buf.b_ml.ml_line_count
        };
        buf_signcols_count_range(
            &mut *buf,
            start_row,
            (count - 1).min(start_row + old_row),
            0,
            K_TRUE,
        );
        buf.b_prev_line_count = 0;
    }

    marktree_splice(
        &mut buf.b_marktree,
        start_row,
        start_col,
        old_row,
        old_col,
        new_row,
        new_col,
    );

    if old_row > 0 || new_row > 0 {
        let row2 = (buf.b_ml.ml_line_count - 1).min(start_row + new_row);
        buf_signcols_count_range(&mut *buf, start_row, row2, 0, K_NONE);
    }

    if undo == K_EXTMARK_UNDO {
        let Some(uh) = u_force_get_undo_header(buf) else {
            return;
        };

        let mut merged = false;
        if old_row == 0
            && new_row == 0
            && let Some(ExtmarkUndoObject::Splice(splice)) =
                crate::undo::uhp_mut(buf, uh).uh_extmark.last_mut()
            && splice.start_row == start_row
            && splice.old_row == 0
            && splice.new_row == 0
        {
            if old_col == 0
                && start_col >= splice.start_col
                && start_col <= splice.start_col + splice.new_col
            {
                splice.new_col += new_col;
                splice.new_byte += new_byte;
                merged = true;
            } else if new_col == 0 && start_col == splice.start_col + splice.new_col {
                splice.old_col += old_col;
                splice.old_byte += old_byte;
                merged = true;
            } else if new_col == 0 && start_col + old_col == splice.start_col {
                splice.start_col = start_col;
                splice.start_byte = start_byte;
                splice.old_col += old_col;
                splice.old_byte += old_byte;
                merged = true;
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
            crate::undo::uhp_mut(buf, uh)
                .uh_extmark
                .push(ExtmarkUndoObject::Splice(splice));
        }
    }
}

pub fn extmark_splice_cols(
    buf: &mut FileBuf,
    start_row: c_int,
    start_col: Colnr,
    old_col: Colnr,
    new_col: Colnr,
    undo: ExtmarkOp,
) {
    extmark_splice(
        buf,
        start_row,
        start_col,
        0,
        old_col,
        old_col as Bcount,
        0,
        new_col,
        new_col as Bcount,
        undo,
    );
}

pub fn extmark_move_region(
    buf: &mut FileBuf,
    start_row: c_int,
    start_col: Colnr,
    start_byte: Bcount,
    extent_row: c_int,
    extent_col: Colnr,
    extent_byte: Bcount,
    new_row: c_int,
    new_col: Colnr,
    new_byte: Bcount,
    undo: ExtmarkOp,
) {
    buf.deleted_bytes2 = 0;
    buf_updates_send_splice(
        &mut *buf,
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
    buf_signcols_count_range(&mut *buf, row1, row2, 0, K_TRUE);

    marktree_move_region(
        &mut buf.b_marktree,
        start_row,
        start_col,
        extent_row,
        extent_col,
        new_row,
        new_col,
    );

    buf_signcols_count_range(&mut *buf, row1, row2, 0, K_NONE);

    buf_updates_send_splice(
        &mut *buf,
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

    if undo == K_EXTMARK_UNDO {
        let Some(uh) = u_force_get_undo_header(&mut *buf) else {
            return;
        };

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
        crate::undo::uhp_mut(&mut *buf, uh)
            .uh_extmark
            .push(ExtmarkUndoObject::Move(move_));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extmark_undo_vec_and_map_destroy_work() {
        let v: ExtmarkUndoVec = vec![ExtmarkUndoObject::SavePos(ExtmarkSavePos {
            mark: 10,
            undo_row: 1,
            undo_col: 2,
            redo_row: 3,
            redo_col: 4,
            invalidated: false,
        })];
        assert_eq!(v.len(), 1);
        drop(v);
    }
}
