#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "klib/kvec.h"
#include "nvim/extmark_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/marktree_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep

EXTERN int curbuf_splice_pending INIT( = 0);

typedef kvec_t(MTPair) ExtmarkInfoArray;

// delete the columns between mincol and endcol
typedef struct {
  int start_row;
  colnr_T start_col;
  int old_row;
  colnr_T old_col;
  int new_row;
  colnr_T new_col;
  bcount_t start_byte;
  bcount_t old_byte;
  bcount_t new_byte;
} ExtmarkSplice;

// adjust marks after :move operation
typedef struct {
  int start_row;
  int start_col;
  int extent_row;
  int extent_col;
  int new_row;
  int new_col;
  bcount_t start_byte;
  bcount_t extent_byte;
  bcount_t new_byte;
} ExtmarkMove;

// extmark was updated
typedef struct {
  uint64_t mark;  // raw mark id of the marktree
  int undo_row;
  colnr_T undo_col;
  int redo_row;   // -1: nothing to redo (the mark was not explicitly moved).
  colnr_T redo_col;
  bool invalidated;
} ExtmarkSavePos;

typedef enum {
  kExtmarkSplice,
  kExtmarkMove,
  kExtmarkUpdate,
  kExtmarkSavePos,
  kExtmarkClear,
} UndoObjectType;

// TODO(bfredl): if possible unify these with marktree flags,
// so it is possible to filter extmarks directly on top-level flags
typedef enum {
  kExtmarkNone = 0x1,
  kExtmarkSign = 0x2,
  kExtmarkSignHL = 0x4,
  kExtmarkVirtText = 0x8,
  kExtmarkVirtLines = 0x10,
  kExtmarkHighlight = 0x20,
} ExtmarkType;

// TODO(bfredl): reduce the number of undo action types
struct undo_object {
  UndoObjectType type;
  union {
    ExtmarkSplice splice;
    ExtmarkMove move;
    ExtmarkSavePos savepos;
  } data;
};

#define DEFINE_FUNC_ATTRIBUTES
#include "nvim/func_attr.h"
#undef DEFINE_FUNC_ATTRIBUTES
#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void extmark_set(buf_T *buf, uint32_t ns_id, uint32_t *idp, int row, colnr_T col,
                           int end_row, colnr_T end_col, DecorInline decor, uint16_t decor_flags,
                           bool right_gravity, bool end_right_gravity, bool no_undo,
                           bool invalidate, Error *err);
DLLEXPORT bool extmark_del_id(buf_T *buf, uint32_t ns_id, uint32_t id);
DLLEXPORT void extmark_del(buf_T *buf, MarkTreeIter *itr, MTKey key, bool restore);
DLLEXPORT bool extmark_clear(buf_T *buf, uint32_t ns_id, int l_row, colnr_T l_col, int u_row,
                             colnr_T u_col);
DLLEXPORT ExtmarkInfoArray extmark_get(buf_T *buf, uint32_t ns_id, int l_row, colnr_T l_col,
                                       int u_row, colnr_T u_col, int64_t amount,
                                       ExtmarkType type_filter, bool overlap);
DLLEXPORT MTPair extmark_from_id(buf_T *buf, uint32_t ns_id, uint32_t id);
DLLEXPORT void extmark_set_pos(buf_T *buf, uint32_t ns_id, uint32_t *mark, pos_T pos,
                               bool right_gravity, bool no_undo, bool ui_watched);
DLLEXPORT bool extmark_get_pos(buf_T *buf, uint32_t ns_id, uint32_t id, pos_T *pos);
DLLEXPORT void extmark_free_all(buf_T *buf);
DLLEXPORT void extmark_splice_delete(buf_T *buf, int l_row, colnr_T l_col, int u_row, colnr_T u_col,
                                     extmark_undo_vec_t *uvp, bool only_copy, ExtmarkOp op);
DLLEXPORT void extmark_apply_undo(ExtmarkUndoObject undo_info, bool undo);
DLLEXPORT void extmark_adjust(buf_T *buf, linenr_T line1, linenr_T line2, linenr_T amount,
                              linenr_T amount_after, ExtmarkOp undo);
DLLEXPORT void extmark_splice(buf_T *buf, int start_row, colnr_T start_col, int old_row,
                              colnr_T old_col, bcount_t old_byte, int new_row, colnr_T new_col,
                              bcount_t new_byte, ExtmarkOp undo);
DLLEXPORT void extmark_splice_impl(buf_T *buf, int start_row, colnr_T start_col,
                                   bcount_t start_byte, int old_row, colnr_T old_col,
                                   bcount_t old_byte, int new_row, colnr_T new_col,
                                   bcount_t new_byte, ExtmarkOp undo);
DLLEXPORT void extmark_splice_cols(buf_T *buf, int start_row, colnr_T start_col, colnr_T old_col,
                                   colnr_T new_col, ExtmarkOp undo);
DLLEXPORT void extmark_move_region(buf_T *buf, int start_row, colnr_T start_col,
                                   bcount_t start_byte, int extent_row, colnr_T extent_col,
                                   bcount_t extent_byte, int new_row, colnr_T new_col,
                                   bcount_t new_byte, ExtmarkOp undo);

#include "nvim/func_attr.h"
