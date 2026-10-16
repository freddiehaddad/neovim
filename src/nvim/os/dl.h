#pragma once

#include <stdbool.h>

bool os_libcall(const char *libname, const char *funcname, const char *argv, int argi,
                char **str_out, int *int_out);
