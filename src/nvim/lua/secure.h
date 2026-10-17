#pragma once

#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep

char *nlua_read_secure(const char *path);
void ex_trust(exarg_T *eap);
