#pragma once

#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/event/defs.h"
#include "nvim/macros_defs.h"
#include "nvim/memory_defs.h"  // IWYU pragma: keep
#include "nvim/msgpack_rpc/channel_defs.h"  // IWYU pragma: keep

#define METHOD_MAXLEN 512

/// HACK: os/input.c drains this queue immediately before blocking for input.
///       Events on this queue are async-safe, but they need the resolved state
///       of input_get(), so they are processed "just-in-time".
EXTERN MultiQueue *ch_before_blocking_events INIT( = NULL);

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

DLLEXPORT void rpc_init(void);
DLLEXPORT void rpc_start(Channel *channel);
DLLEXPORT bool rpc_send_event(uint64_t id, const char *name, Array args);
DLLEXPORT Object rpc_send_call(uint64_t id, const char *method_name, Array args,
                               ArenaMem *result_mem, Error *err);
DLLEXPORT bool rpc_write_raw(uint64_t id, WBuffer *buffer);
DLLEXPORT void rpc_close(Channel *channel);
DLLEXPORT void rpc_free(Channel *channel);
DLLEXPORT void serialize_response(Channel *channel, MsgpackRpcRequestHandler handler,
                                  MessageType type, uint32_t response_id, Error *err, Object *arg);
DLLEXPORT void rpc_set_client_info(uint64_t id, Dict info);
DLLEXPORT Dict rpc_client_info(Channel *chan);
DLLEXPORT const char *get_client_info(Channel *chan, const char *key) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void rpc_free_all_mem(void);

#include "nvim/func_attr.h"
