#pragma once

#include <stdio.h>  // IWYU pragma: keep

#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void ex_loadview(exarg_T *eap);
DLLEXPORT void ex_mkrc(exarg_T *eap);
DLLEXPORT int put_eol(FILE *fd);
DLLEXPORT int put_line(FILE *fd, char *s);
