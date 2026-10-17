#pragma once

#include "nvim/func_attr.h"
#include "nvim/normal_defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/vim_defs.h"  // IWYU pragma: keep

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT int findsent(Direction dir, int count);
DLLEXPORT bool findpar(bool *pincl, int dir, int count, int what, bool both);
DLLEXPORT bool startPS(linenr_T lnum, int para, bool both);
DLLEXPORT int fwd_word(int count, bool bigword, bool eol);
DLLEXPORT int bck_word(int count, bool bigword, bool stop);
DLLEXPORT int end_word(int count, bool bigword, bool stop, bool empty);
DLLEXPORT int bckend_word(int count, bool bigword, bool eol);
DLLEXPORT int current_word(oparg_T *oap, int count, bool include, bool bigword);
DLLEXPORT int current_line(oparg_T *oap, bool include);
DLLEXPORT int current_sent(oparg_T *oap, int count, bool include);
DLLEXPORT int current_block(oparg_T *oap, int count, bool include, int what, int other);
DLLEXPORT int current_tagblock(oparg_T *oap, int count_arg, bool include);
DLLEXPORT int current_par(oparg_T *oap, int count, bool include, int type);
DLLEXPORT bool current_quote(oparg_T *oap, int count, bool include, int quotechar)
  FUNC_ATTR_NONNULL_ALL;
