#pragma once

#include <stddef.h>
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

DLLEXPORT void rstream_init_fd(Loop *loop, RStream *stream, int fd)
  FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT void rstream_init_stream(RStream *stream, uv_stream_t *uvstream)
  FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT void rstream_init(RStream *stream) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void rstream_start_inner(RStream *stream) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void rstream_start(RStream *stream, stream_read_cb cb, void *data)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void rstream_stop_inner(RStream *stream) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void rstream_stop(RStream *stream) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT size_t rstream_available(RStream *stream);
DLLEXPORT void rstream_consume(RStream *stream, size_t consumed);
DLLEXPORT void rstream_may_close(RStream *stream);
