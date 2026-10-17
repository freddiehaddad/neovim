#pragma once

#include <stddef.h>  // IWYU pragma: keep

#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/os/time_defs.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// Present history tables
typedef enum {
  HIST_DEFAULT = -2,  ///< Default (current) history.
  HIST_INVALID = -1,  ///< Unknown history.
  HIST_CMD = 0,       ///< Colon commands.
  HIST_SEARCH,        ///< Search commands.
  HIST_EXPR,          ///< Expressions (e.g. from entering = register).
  HIST_INPUT,         ///< input() lines.
  HIST_DEBUG,         ///< Debug commands.
} HistoryType;

enum { HIST_COUNT = HIST_DEBUG + 1, };  ///< Number of history tables

/// History entry definition
typedef struct {
  int hisnum;           ///< Entry identifier number.
  char *hisstr;         ///< Actual entry, separator char after the NUL.
  size_t hisstrlen;     ///< Length of hisstr (excluding the NUL).
  Timestamp timestamp;  ///< Time when entry was added.
  AdditionalData *additional_data;  ///< Additional entries from ShaDa file.
} histentry_T;

int get_hislen(void);
histentry_T *get_histentry(int hist_type);
void set_histentry(int hist_type, histentry_T *entry);
int *get_hisidx(int hist_type);
int *get_hisnum(int hist_type);
HistoryType hist_char2type(const int c);
char *get_history_arg(expand_T *xp, int idx);
void init_history(void);
void add_to_history(int histype, const char *new_entry, size_t new_entrylen, bool in_map, int sep);
int clr_history(const int histype);
void f_histadd(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_histdel(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_histget(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_histnr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void ex_history(exarg_T *eap);
const void *hist_iter(const void *const iter, const uint8_t history_type, const bool zero,
                      histentry_T *const hist);
histentry_T *hist_get_array(const uint8_t history_type, int **const new_hisidx,
                            int **const new_hisnum);
