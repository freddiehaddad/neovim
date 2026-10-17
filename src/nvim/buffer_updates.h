#pragma once

#include <stdbool.h>
#include <stdint.h>  // IWYU pragma: keep

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/extmark_defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"  // IWYU pragma: keep

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

DLLEXPORT bool buf_updates_register(buf_T *buf, uint64_t channel_id, BufUpdateCallbacks cb,
                                    bool send_buffer);
DLLEXPORT bool buf_updates_active(buf_T *buf) FUNC_ATTR_PURE;
DLLEXPORT void buf_updates_send_end(buf_T *buf, uint64_t channelid);
DLLEXPORT void buf_updates_unregister(buf_T *buf, uint64_t channelid);
DLLEXPORT void buf_free_callbacks(buf_T *buf);
DLLEXPORT void buf_updates_unload(buf_T *buf, bool can_reload);
DLLEXPORT void buf_updates_send_changes(buf_T *buf, linenr_T firstline, int64_t num_added,
                                        int64_t num_removed);
DLLEXPORT void buf_updates_send_splice(buf_T *buf, int start_row, colnr_T start_col,
                                       bcount_t start_byte, int old_row, colnr_T old_col,
                                       bcount_t old_byte, int new_row, colnr_T new_col,
                                       bcount_t new_byte);
DLLEXPORT void buf_updates_changedtick(buf_T *buf);
DLLEXPORT void buf_updates_changedtick_single(buf_T *buf, uint64_t channel_id);
DLLEXPORT void buffer_update_callbacks_free(BufUpdateCallbacks cb);

#include "nvim/func_attr.h"
