#pragma once

#include <stdbool.h>
#include <uv.h>

#include "nvim/event/defs.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT int stream_set_blocking(int fd, bool blocking);
DLLEXPORT void stream_init(Loop *loop, Stream *stream, int fd, uv_stream_t *uvstream)
  FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT void stream_may_close(Stream *stream) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void stream_close_handle(Stream *stream) FUNC_ATTR_NONNULL_ALL;
