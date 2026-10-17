#pragma once

#include <stdbool.h>
#include <stddef.h>  // IWYU pragma: keep

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT bool server_init(const char *listen_addr);
DLLEXPORT void server_teardown(void);
DLLEXPORT char *server_address_new(const char *name)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_RET;
DLLEXPORT bool server_owns_pipe_address(const char *address);
DLLEXPORT int server_start(const char *addr);
DLLEXPORT bool server_stop(const char *endpoint, bool keep_vservername);
DLLEXPORT char **server_address_list(size_t *size) FUNC_ATTR_NONNULL_ALL;
