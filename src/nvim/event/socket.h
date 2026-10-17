#pragma once

#include <stdbool.h>

#include "nvim/event/defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

#define DEFINE_FUNC_ATTRIBUTES
#include "nvim/func_attr.h"
#undef DEFINE_FUNC_ATTRIBUTES
#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT char *socket_address_tcp_host_end(const char *address);
DLLEXPORT int socket_watcher_init(Loop *loop, SocketWatcher *watcher,
                                  const char *endpoint) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int socket_watcher_start(SocketWatcher *watcher, int backlog,
                                   socket_cb cb) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int socket_watcher_accept(SocketWatcher *watcher,
                                    RStream *stream) FUNC_ATTR_NONNULL_ARG(1)
  FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT void socket_watcher_close(SocketWatcher *watcher,
                                    socket_close_cb cb) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT bool socket_connect(Loop *loop, RStream *stream, bool is_tcp, const char *address,
                              int timeout, const char **error);

#include "nvim/func_attr.h"
