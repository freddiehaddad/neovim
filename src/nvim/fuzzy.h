#pragma once

#include <limits.h>
#include <stdint.h>  // IWYU pragma: keep

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/garray_defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

enum { FUZZY_MATCH_MAX_LEN = 1024, };  ///< max characters that can be matched
enum { FUZZY_SCORE_NONE = INT_MIN, };  ///< invalid fuzzy score

/// Fuzzy matched string list item. Used for fuzzy match completion. Items are
/// usually sorted by "score". The "idx" member is used for stable-sort.
typedef struct {
  int idx;
  char *str;
  int score;
} fuzmatch_str_T;

bool fuzzy_match(char *const str, const char *const pat_arg, const bool matchseq,
                 int *const outScore, uint32_t *const matches, const int maxMatches);
void f_matchfuzzy(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_matchfuzzypos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
int fuzzy_match_str(char *const str, const char *const pat);
garray_T *fuzzy_match_str_with_pos(char *const str, const char *const pat);
bool fuzzy_match_str_in_line(char **ptr, char *pat, int *len, pos_T *current_pos, int *score);
bool search_for_fuzzy_match(buf_T *buf, pos_T *pos, char *pattern, int dir, pos_T *start_pos,
                            int *len, char **ptr, int *score);
void fuzmatch_str_free(fuzmatch_str_T *const fuzmatch, int count);
void fuzzymatches_to_strmatches(fuzmatch_str_T *const fuzmatch, char ***const matches,
                                const int count, const bool funcsort);
