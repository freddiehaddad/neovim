#pragma once

#include <stdbool.h>
#include <stddef.h>  // IWYU pragma: keep

#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "xdiff/xdiff.h"  // IWYU pragma: keep

mmfile_t fastforward_buf_to_lnum(mmfile_t s, linenr_T lnum);
size_t linematch_nbuffers(const mmfile_t **diff_blk, const int *diff_len, const size_t ndiffs,
                          int **decisions, bool iwhite);
