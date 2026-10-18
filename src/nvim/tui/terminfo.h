#pragma once

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/tui/terminfo_defs.h"

typedef struct {
  long num;
  char *string;
} TPVAR;

#include "nvim/func_attr.h"

DLLEXPORT bool terminfo_is_term_family(const char *term, const char *family);
DLLEXPORT bool terminfo_is_bsd_console(const char *term);
DLLEXPORT const TerminfoEntry *terminfo_from_builtin(const char *term, char **termname);
DLLEXPORT bool terminfo_from_database(TerminfoEntry *ti, char *termname, Arena *arena);
DLLEXPORT String terminfo_info_msg(const TerminfoEntry *ti, const char *termname, bool from_db);
DLLEXPORT size_t terminfo_fmt(char *buf_start, char *buf_end, const char *str, TPVAR params[9]);
