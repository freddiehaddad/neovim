#pragma once

#include <stdbool.h>

#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"  // IWYU pragma: keep

void do_debug(char *cmd);
void ex_debug(exarg_T *eap);
void dbg_check_breakpoint(exarg_T *eap);
bool dbg_check_skipped(exarg_T *eap);
void ex_breakadd(exarg_T *eap);
void ex_debuggreedy(exarg_T *eap);
void ex_breakdel(exarg_T *eap);
void ex_breaklist(exarg_T *eap);
linenr_T dbg_find_breakpoint(bool file, char *fname, linenr_T after);
bool has_profiling(bool file, char *fname, bool *fp);
void dbg_breakpoint(char *name, linenr_T lnum);
