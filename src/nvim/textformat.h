#pragma once

#include <stdbool.h>

#include "nvim/func_attr.h"
#include "nvim/normal_defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"  // IWYU pragma: keep

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT bool has_format_option(int x) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void internal_format(int textwidth, int second_indent, int flags, bool format_only,
                               int c);
DLLEXPORT void auto_format(bool trailblank, bool prev_line);
DLLEXPORT void check_auto_format(bool end_insert);
DLLEXPORT int comp_textwidth(bool ff);
DLLEXPORT void op_format(oparg_T *oap, bool keep_cursor);
DLLEXPORT void op_formatexpr(oparg_T *oap);
DLLEXPORT int fex_format(linenr_T lnum, long count, int c);
DLLEXPORT void format_lines(linenr_T line_count, bool avoid_fex);
