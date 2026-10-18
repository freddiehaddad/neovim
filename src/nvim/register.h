#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "nvim/api/private/defs.h"
#include "nvim/ascii_defs.h"
#include "nvim/ex_cmds_defs.h"
#include "nvim/func_attr.h"
#include "nvim/macros_defs.h"
#include "nvim/normal_defs.h"
#include "nvim/os/time_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/register_defs.h"
#include "nvim/types_defs.h"

/// Used by TextPutPost/TextPutPre autocommands for the '.' register. If
/// "add_last_insert" is == 1, then "stuff_inserted" will add the last inserted
/// text to "last_insert_ga".
EXTERN garray_T last_insert_ga INIT( = { 0, 0, 1, 64, NULL });
EXTERN int add_last_insert INIT( = 0);

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

DLLEXPORT int get_unname_register(void);
DLLEXPORT yankreg_T *get_y_register(int reg);
DLLEXPORT yankreg_T *get_y_previous(void) FUNC_ATTR_PURE;
DLLEXPORT int get_expr_register(void);
DLLEXPORT void set_expr_line(char *new_line);
DLLEXPORT char *get_expr_line(void);
DLLEXPORT char *get_expr_line_src(void);
DLLEXPORT bool valid_yank_reg(int regname, bool writing);
DLLEXPORT int get_default_register_name(void);
DLLEXPORT const void *op_reg_iter(const void *const iter, const yankreg_T *const regs,
                                  char *const name, yankreg_T *const reg, bool *is_unnamed)
  FUNC_ATTR_NONNULL_ARG(3, 4, 5) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT const void *op_global_reg_iter(const void *const iter, char *const name,
                                         yankreg_T *const reg, bool *is_unnamed)
  FUNC_ATTR_NONNULL_ARG(2, 3, 4) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT Timestamp reg_max_ts(bool clipboard);
DLLEXPORT size_t op_reg_amount(void) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool op_reg_set(const char name, const yankreg_T reg, bool is_unnamed);
DLLEXPORT const yankreg_T *op_reg_get(const char name);
DLLEXPORT bool op_reg_set_previous(const char name);
DLLEXPORT void update_yankreg_width(yankreg_T *reg);
DLLEXPORT yankreg_T *get_yank_register(int regname, int mode);
DLLEXPORT bool yank_register_mline(int regname, yankreg_T **reg);
DLLEXPORT yankreg_T *copy_register(int name) FUNC_ATTR_NONNULL_RET;
DLLEXPORT int do_record(int c);
DLLEXPORT int do_execreg(int regname, int colon, int addcr, int silent);
DLLEXPORT int insert_reg(int regname, yankreg_T *reg, bool literally_arg);
DLLEXPORT bool get_spec_reg(int regname, char **argp, bool *allocated, bool errmsg);
DLLEXPORT bool cmdline_paste_reg(int regname, bool literally_arg, bool remcr);
DLLEXPORT void shift_delete_registers(bool y_append);
DLLEXPORT void clear_registers(void);
DLLEXPORT void free_register(yankreg_T *reg) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void op_yank_reg(oparg_T *oap, bool message, yankreg_T *reg, bool append);
DLLEXPORT size_t format_reg_type(MotionType reg_type, colnr_T reg_width, char *buf, size_t bufsize)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void do_autocmd_textyankpost(oparg_T *oap, yankreg_T *reg) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool op_yank(oparg_T *oap, bool message) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void do_put(int regname, yankreg_T *reg, int dir, int count, int flags);
DLLEXPORT void ex_display(exarg_T *eap);
DLLEXPORT MotionType get_reg_type(int regname, colnr_T *reg_width);
DLLEXPORT void *get_reg_contents(int regname, int flags);
DLLEXPORT void write_reg_contents(int name, const char *str, ssize_t len, int must_append);
DLLEXPORT void write_reg_contents_lst(int name, char **strings, bool must_append,
                                      MotionType yank_type, colnr_T block_len);
DLLEXPORT void write_reg_contents_ex(int name, const char *str, ssize_t len, bool must_append,
                                     MotionType yank_type, colnr_T block_len);
DLLEXPORT bool prepare_yankreg_from_object(yankreg_T *reg, String regtype, size_t lines);
DLLEXPORT void finish_yankreg_from_object(yankreg_T *reg, bool clipboard_adjust);

#include "nvim/func_attr.h"

#include "register.h.inline.generated.h"

/// @see get_yank_register
/// @return  true when register should be inserted literally
/// (selection or clipboard)
static inline bool is_literal_register(const int regname)
  FUNC_ATTR_CONST
{
  return regname == '*' || regname == '+' || ASCII_ISALNUM(regname);
}

/// Convert register name into register index
///
/// @param[in]  regname  Register name.
///
/// @return Index in y_regs array or -1 if register name was not recognized.
static inline int op_reg_index(const int regname)
  FUNC_ATTR_CONST
{
  if (ascii_isdigit(regname)) {
    return regname - '0';
  } else if (ASCII_ISLOWER(regname)) {
    return CHAR_ORD_LOW(regname) + 10;
  } else if (ASCII_ISUPPER(regname)) {
    return CHAR_ORD_UP(regname) + 10;
  } else if (regname == '-') {
    return DELETION_REGISTER;
  } else if (regname == '*') {
    return STAR_REGISTER;
  } else if (regname == '+') {
    return PLUS_REGISTER;
  } else {
    return -1;
  }
}

static inline bool is_append_register(int regname)
  FUNC_ATTR_CONST
{
  return ASCII_ISUPPER(regname);
}

/// @return  the character name of the register with the given number
static inline int get_register_name(int num)
  FUNC_ATTR_CONST
{
  if (num == -1) {
    return '"';
  } else if (num < 10) {
    return num + '0';
  } else if (num == DELETION_REGISTER) {
    return '-';
  } else if (num == STAR_REGISTER) {
    return '*';
  } else if (num == PLUS_REGISTER) {
    return '+';
  } else {
    return num + 'a' - 10;
  }
}

/// Check whether register is empty
static inline bool reg_empty(const yankreg_T *const reg)
  FUNC_ATTR_PURE
{
  return (reg->y_array == NULL
          || reg->y_size == 0
          || (reg->y_size == 1
              && reg->y_type == kMTCharWise
              && reg->y_array[0].size == 0));
}
