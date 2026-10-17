#pragma once

#include <stdbool.h>
#include <stddef.h>  // IWYU pragma: keep
#include <stdint.h>

#include "klib/kvec.h"
#include "nvim/decoration_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/marktree_defs.h"
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/sign_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"

// actual Decor* data is in decoration_defs.h

/// Keep in sync with VirtTextPos in decoration_defs.h
EXTERN const char *const virt_text_pos_str[] INIT( = { "eol", "eol_right_align", "inline",
                                                       "overlay", "right_align", "win_col" });

/// Keep in sync with HlMode in decoration_defs.h
EXTERN const char *const hl_mode_str[] INIT( = { "", "replace", "combine", "blend" });

typedef enum {
  kDecorKindHighlight,
  kDecorKindSign,
  kDecorKindVirtText,
  kDecorKindVirtLines,
  kDecorKindUIWatched,
} DecorRangeKindEnum;

typedef uint8_t DecorRangeKind;

typedef struct {
  int start_row;
  int start_col;
  int end_row;
  int end_col;
  int ordering;  ///< range insertion order
  DecorPriorityInternal priority_internal;
  bool owned;  ///< ephemeral decoration, free memory immediately
  DecorRangeKind kind;
  // next pointers MUST NOT be used, these are separate ranges
  // vt->next could be pointing to freelist memory at this point
  union {
    DecorSignHighlight sh;
    DecorVirtText *vt;
    struct {
      uint32_t ns_id;
      uint32_t mark_id;
      VirtTextPos pos;
    } ui;
  } data;
  int attr_id;  ///< cached lookup of inl.hl_id if it was a highlight
  /// Screen column to draw the virtual text.
  /// When -1, it should be drawn on the current screen line after deciding where.
  /// When -3, it may be drawn at a position yet to be assigned.
  /// When -10, it has just been added.
  /// When INT_MIN, it should no longer be drawn.
  int draw_col;
} DecorRange;

/// DecorRange can be removed from `DecorState` list in any order,
/// so we track available slots using a freelist (with `next_free_i`).
/// The list head is in `DecorState.free_slot_i`.
typedef union {
  DecorRange range;
  int next_free_i;
} DecorRangeSlot;

typedef struct {
  MarkTreeIter itr[1];
  kvec_t(DecorRangeSlot) slots;
  kvec_t(int) ranges_i;
  /// Indices in [0; current_end) of `ranges_i` point to ranges that start
  /// before current position. Sorted by priority and order of insertion.
  int current_end;
  /// Indices in [future_begin, kv_size(ranges_i)) of `ranges_i` point to
  /// ranges that start after current position. Sorted by starting position.
  int future_begin;
  /// Head of DecorRangeSlot freelist. -1 if none are freed.
  int free_slot_i;
  /// Index for keeping track of range insertion order.
  int new_range_ordering;
  win_T *win;
  int top_row;
  int row;
  int col_last;
  int current;
  int current_hl_eol;  ///< "current" limited to ranges with "hl_eol"
  int eol_col;

  int conceal;
  schar_T conceal_char;
  int conceal_attr;

  TriState spell;

  bool running_decor_provider;
  bool itr_valid;
} DecorState;

EXTERN DecorState decor_state INIT( = { 0 });
// TODO(bfredl): These should maybe be per-buffer, so that all resources
// associated with a buffer can be freed when the buffer is unloaded.
EXTERN kvec_t(DecorSignHighlight) decor_items INIT( = KV_INITIAL_VALUE);

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void bufhl_add_hl_pos_offset(buf_T *buf, int src_id, int hl_id, lpos_T pos_start,
                                       lpos_T pos_end, colnr_T offset);
DLLEXPORT void decor_redraw(buf_T *buf, int row1, int row2, int col1, DecorInline decor);
DLLEXPORT void decor_redraw_sh(buf_T *buf, int row1, int row2, DecorSignHighlight sh);
DLLEXPORT uint32_t decor_put_sh(DecorSignHighlight item);
DLLEXPORT DecorVirtText *decor_put_vt(DecorVirtText vt, DecorVirtText *next);
DLLEXPORT DecorSignHighlight decor_sh_from_inline(DecorHighlightInline item);
DLLEXPORT void buf_put_decor(buf_T *buf, DecorInline decor, int row, int row2);
DLLEXPORT void buf_put_decor_sh(buf_T *buf, DecorSignHighlight *sh, int row1, int row2);
DLLEXPORT void buf_decor_remove(buf_T *buf, int row1, int row2, int col1, DecorInline decor,
                                bool free);
DLLEXPORT void buf_remove_decor_sh(buf_T *buf, int row1, int row2, DecorSignHighlight *sh);
DLLEXPORT void decor_free(DecorInline decor);
DLLEXPORT void decor_state_invalidate(buf_T *buf);
DLLEXPORT void decor_check_to_be_deleted(void);
DLLEXPORT void decor_state_free(DecorState *state);
DLLEXPORT void clear_virttext(VirtText *text);
DLLEXPORT void clear_virtlines(VirtLines *lines);
DLLEXPORT void decor_check_invalid_glyphs(void);
DLLEXPORT char *next_virt_text_chunk(VirtText vt, size_t *pos, int *attr);
DLLEXPORT DecorVirtText *decor_find_virttext(buf_T *buf, int row, uint64_t ns_id);
DLLEXPORT bool decor_redraw_reset(win_T *wp, DecorState *state);
DLLEXPORT bool decor_virt_pos(const DecorRange *decor);
DLLEXPORT VirtTextPos decor_virt_pos_kind(const DecorRange *decor);
DLLEXPORT bool decor_redraw_start(win_T *wp, int top_row, DecorState *state);
DLLEXPORT void decor_redraw_line(win_T *wp, int row, DecorState *state);
DLLEXPORT bool decor_has_more_decorations(DecorState *state, int row);
DLLEXPORT void decor_range_add_virt(DecorState *state, int start_row, int start_col, int end_row,
                                    int end_col, DecorVirtText *vt, bool owned);
DLLEXPORT void decor_range_add_sh(DecorState *state, int start_row, int start_col, int end_row,
                                  int end_col, DecorSignHighlight *sh, bool owned, uint32_t ns,
                                  uint32_t mark_id, DecorPriority subpriority);
DLLEXPORT void decor_init_draw_col(int win_col, bool hidden, DecorRange *item);
DLLEXPORT void decor_recheck_draw_col(int win_col, bool hidden, DecorState *state);
DLLEXPORT int decor_redraw_col_impl(win_T *wp, int col, int win_col, bool hidden, DecorState *state,
                                    int max_col_last);
DLLEXPORT bool decor_conceal_line(win_T *wp, int row, bool check_cursor);
DLLEXPORT bool win_lines_concealed(win_T *wp);
DLLEXPORT int sign_item_cmp(const void *p1, const void *p2);
DLLEXPORT void decor_redraw_signs(win_T *wp, buf_T *buf, int row, SignTextAttrs sattrs[],
                                  int *line_id, int *cul_id, int *num_id);
DLLEXPORT DecorSignHighlight *decor_find_sign(DecorInline decor);
DLLEXPORT void buf_signcols_count_range(buf_T *buf, int row1, int row2, int add, TriState clear);
DLLEXPORT void decor_redraw_end(DecorState *state);
DLLEXPORT bool decor_redraw_eol(win_T *wp, DecorState *state, int *eol_attr, int eol_col);
DLLEXPORT int decor_virt_line_rows(win_T *wp, const struct virt_line *vl, int target_row,
                                   int *skip_cells);
DLLEXPORT int decor_virt_lines(win_T *wp, int start_row, int end_row, int *num_below,
                               VirtLines *lines, bool apply_folds);
DLLEXPORT void decor_to_dict_legacy(Dict *dict, DecorInline decor, bool hl_name, Arena *arena);
DLLEXPORT uint16_t decor_type_flags(DecorInline decor);
DLLEXPORT Object hl_group_name(int hl_id, bool hl_name);

#include "decoration.h.inline.generated.h"

static inline int decor_redraw_col(win_T *wp, int col, int win_col, bool hidden, DecorState *state,
                                   int max_col_last)
  FUNC_ATTR_ALWAYS_INLINE
{
  if (col <= state->col_last) {
    return state->current;
  }
  return decor_redraw_col_impl(wp, col, win_col, hidden, state, max_col_last);
}
