#pragma once

#include <lauxlib.h>  // for LUA_NOREF
#include <stdbool.h>
#include <stddef.h>

#include "nvim/api/private/defs.h"
#include "nvim/ascii_defs.h"
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/extmark_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/normal_defs.h"
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/os/time_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/register_defs.h"
#include "nvim/types_defs.h"

/// Operator IDs; The order must correspond to opchars[] in ops.c!
enum {
  OP_NOP          = 0,   ///< no pending operation
  OP_DELETE       = 1,   ///< "d"  delete operator
  OP_YANK         = 2,   ///< "y"  yank operator
  OP_CHANGE       = 3,   ///< "c"  change operator
  OP_LSHIFT       = 4,   ///< "<"  left shift operator
  OP_RSHIFT       = 5,   ///< ">"  right shift operator
  OP_FILTER       = 6,   ///< "!"  filter operator
  OP_TILDE        = 7,   ///< "g~" switch case operator
  OP_INDENT       = 8,   ///< "="  indent operator
  OP_FORMAT       = 9,   ///< "gq" format operator
  OP_COLON        = 10,  ///< ":"  colon operator
  OP_UPPER        = 11,  ///< "gU" make upper case operator
  OP_LOWER        = 12,  ///< "gu" make lower case operator
  OP_JOIN         = 13,  ///< "J"  join operator, only for Visual mode
  OP_JOIN_NS      = 14,  ///< "gJ"  join operator, only for Visual mode
  OP_ROT13        = 15,  ///< "g?" rot-13 encoding
  OP_REPLACE      = 16,  ///< "r"  replace chars, only for Visual mode
  OP_INSERT       = 17,  ///< "I"  Insert column, only for Visual mode
  OP_APPEND       = 18,  ///< "A"  Append column, only for Visual mode
  OP_FOLD         = 19,  ///< "zf" define a fold
  OP_FOLDOPEN     = 20,  ///< "zo" open folds
  OP_FOLDOPENREC  = 21,  ///< "zO" open folds recursively
  OP_FOLDCLOSE    = 22,  ///< "zc" close folds
  OP_FOLDCLOSEREC = 23,  ///< "zC" close folds recursively
  OP_FOLDDEL      = 24,  ///< "zd" delete folds
  OP_FOLDDELREC   = 25,  ///< "zD" delete folds recursively
  OP_FORMAT2      = 26,  ///< "gw" format operator, keeps cursor pos
  OP_FUNCTION     = 27,  ///< "g@" call 'operatorfunc'
  OP_NR_ADD       = 28,  ///< "<C-A>" Add to the number or alphabetic character
  OP_NR_SUB       = 29,  ///< "<C-X>" Subtract from the number or alphabetic character
  OP_MCURSOR      = 30,  ///< "zq" place a multicursor at (repeated) motion.
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

DLLEXPORT int get_op_type(int char1, int char2);
DLLEXPORT int op_on_lines(int op);
DLLEXPORT int op_is_change(int op);
DLLEXPORT bool op_redoable(int op, bool redo_yank);
DLLEXPORT int get_op_char(int optype);
DLLEXPORT int get_extra_op_char(int optype);
DLLEXPORT void op_shift(oparg_T *oap, bool curs_top, int amount);
DLLEXPORT void shift_line(bool left, bool round, int amount, int call_changed_bytes);
DLLEXPORT int op_delete(oparg_T *oap);
DLLEXPORT void op_tilde(oparg_T *oap);
DLLEXPORT bool swapchar(int op_type, pos_T *pos) FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT void op_insert(oparg_T *oap, int count1);
DLLEXPORT int op_change(oparg_T *oap);
DLLEXPORT void adjust_cursor_eol(void);
DLLEXPORT char *skip_comment(char *line, bool process, bool include_space, bool *is_comment);
DLLEXPORT int do_join(size_t count, bool insert_space, bool save_undo, bool use_formatoptions,
                      bool setmark);
DLLEXPORT bool reset_lbr(void);
DLLEXPORT void restore_lbr(bool lbr_saved);
DLLEXPORT void block_prep(oparg_T *oap, struct block_def *bdp, linenr_T lnum, bool is_del);
DLLEXPORT void getregionpos_prep(pos_T *p1, pos_T *p2, MotionType region_type, bool exclusive,
                                 int block_width, bool *inclusive, oparg_T *oap);
DLLEXPORT void getregionpos_line(linenr_T lnum, pos_T p1, pos_T p2, bool inclusive,
                                 MotionType region_type, oparg_T *oap, bool allow_eol,
                                 pos_T *ret_p1, pos_T *ret_p2);
DLLEXPORT void charwise_block_prep(pos_T start, pos_T end, struct block_def *bdp, linenr_T lnum,
                                   bool inclusive);
DLLEXPORT void op_addsub(oparg_T *oap, linenr_T Prenum1, bool g_cmd);
DLLEXPORT bool do_addsub(int op_type, pos_T *pos, int length, linenr_T Prenum1);
DLLEXPORT void clear_oparg(oparg_T *oap);
DLLEXPORT void cursor_pos_info(dict_T *dict);
DLLEXPORT void free_operatorfunc_option(void);
DLLEXPORT bool set_ref_in_opfunc(int copyID);
DLLEXPORT bool op_self_select(const cmdarg_T *cap);
DLLEXPORT void do_pending_operator(cmdarg_T *cap, int old_col, bool gui_yank);
DLLEXPORT bcount_t get_region_bytecount(buf_T *buf, linenr_T start_lnum, linenr_T end_lnum,
                                        colnr_T start_col, colnr_T end_col);

#include "nvim/func_attr.h"

#include "ops.h.inline.generated.h"
