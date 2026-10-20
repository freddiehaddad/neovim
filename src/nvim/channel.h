#pragma once

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#include "nvim/autocmd_defs.h"
#include "nvim/channel_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/event/defs.h"
#include "nvim/event/libuv_proc.h"
#include "nvim/macros_defs.h"
#include "nvim/map_defs.h"
#include "nvim/msgpack_rpc/channel_defs.h"
#include "nvim/os/pty_proc.h"
#include "nvim/types_defs.h"

struct Channel {
  uint64_t id;
  size_t refcount;
  MultiQueue *events;

  ChannelStreamType streamtype;
  union {
    Proc proc;
    LibuvProc uv;
    PtyProc pty;
    RStream socket;
    StdioPair stdio;
    StderrState err;
    InternalState internal;
  } stream;

  bool is_rpc;
  bool detach;  ///< Prevents self-exit on channel-close. Nvim self-exits when the last open
                ///< detach=false RPC channel is closed. The stdio channel defaults to detach=false.
  RpcState rpc;
  Terminal *term;

  CallbackReader on_data;
  CallbackReader on_stderr;
  Callback on_exit;
  int exit_status;  ///< Process exit-code (if the channel wraps a process).

  bool callback_busy;
  bool callback_scheduled;
  bool did_close_event;
};

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

DLLEXPORT void channel_teardown(void);
DLLEXPORT void channel_free_all_mem(void);
DLLEXPORT bool channel_close(uint64_t id, ChannelPart part, const char **error);
DLLEXPORT void channel_init(void);
DLLEXPORT Channel *channel_alloc(ChannelStreamType type) FUNC_ATTR_NONNULL_RET;
DLLEXPORT void channel_create_event(Channel *chan, const char *ext_source);
DLLEXPORT void channel_incref(Channel *chan);
DLLEXPORT void channel_decref(Channel *chan);
DLLEXPORT void callback_reader_free(CallbackReader *reader);
DLLEXPORT void callback_reader_start(CallbackReader *reader, const char *type);
DLLEXPORT Channel *channel_job_start(char **argv, const char *exepath, CallbackReader on_stdout,
                                     CallbackReader on_stderr, Callback on_exit, bool pty, bool rpc,
                                     bool overlapped, bool detach, ChannelStdinMode stdin_mode,
                                     const char *cwd, uint16_t pty_width, uint16_t pty_height,
                                     dict_T *env, varnumber_T *status_out);
DLLEXPORT uint64_t channel_connect(bool tcp, const char *address, bool rpc,
                                   CallbackReader on_output, int timeout, const char **error);
DLLEXPORT void channel_from_connection(SocketWatcher *watcher);
DLLEXPORT uint64_t channel_from_stdio(bool rpc, CallbackReader on_output,
                                      const char **error) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT size_t channel_send(uint64_t id, char *data, size_t len, bool data_owned,
                              const char **error) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT size_t on_channel_data(RStream *stream, const char *buf, size_t count, void *data,
                                 bool eof);
DLLEXPORT size_t on_job_stderr(RStream *stream, const char *buf, size_t count, void *data,
                               bool eof);
DLLEXPORT void channel_reader_callbacks(Channel *chan, CallbackReader *reader);
DLLEXPORT void channel_terminal_alloc(buf_T *buf, Channel *chan);
DLLEXPORT void channel_event(Channel *chan, event_T event);
DLLEXPORT bool channel_job_running(uint64_t id);
DLLEXPORT Dict channel_info(uint64_t id, Arena *arena);
DLLEXPORT Array channel_all_info(Arena *arena);

#include "nvim/func_attr.h"

static inline bool callback_reader_set(CallbackReader reader)
{
  return reader.cb.type != kCallbackNone || reader.self;
}

EXTERN PMap(uint64_t) channels INIT( = MAP_INIT);

EXTERN Callback on_print INIT( = CALLBACK_INIT);

/// @returns Channel with the id or NULL if not found
static inline Channel *find_channel(uint64_t id)
{
  return (Channel *)pmap_get(uint64_t)(&channels, id);
}

static inline Stream *channel_instream(Channel *chan)
  FUNC_ATTR_NONNULL_ALL
{
  switch (chan->streamtype) {
  case kChannelStreamProc:
    return &chan->stream.proc.in;

  case kChannelStreamSocket:
    return &chan->stream.socket.s;

  case kChannelStreamStdio:
    return &chan->stream.stdio.out;

  case kChannelStreamInternal:
  case kChannelStreamStderr:
    abort();
  }
  abort();
}

static inline RStream *channel_outstream(Channel *chan)
  FUNC_ATTR_NONNULL_ALL
{
  switch (chan->streamtype) {
  case kChannelStreamProc:
    return &chan->stream.proc.out;

  case kChannelStreamSocket:
    return &chan->stream.socket;

  case kChannelStreamStdio:
    return &chan->stream.stdio.in;

  case kChannelStreamInternal:
  case kChannelStreamStderr:
    abort();
  }
  abort();
}
