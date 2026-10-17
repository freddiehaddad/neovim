#pragma once

#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep

int eexe_mod_op(typval_T *const tv1, const typval_T *const tv2, const char *const op);
