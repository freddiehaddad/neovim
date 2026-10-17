#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "nvim/buffer_defs.h"  // buf_T
#include "nvim/context_defs.h"  // IWYU pragma: keep (used by mcursor.h.generated.h)
#include "nvim/eval/typval_defs.h"  // varnumber_T
#include "nvim/func_attr.h"
#include "nvim/input_cmdatom_defs.h"  // IWYU pragma: keep (used by mcursor.h.generated.h)
#include "nvim/normal_defs.h"  // IWYU pragma: keep (cmdarg_T, used by mcursor.h.generated.h)
#include "nvim/pos_defs.h"
#include "nvim/register_defs.h"  // IWYU pragma: keep (yankreg_T, used by mcursor.h.generated.h)
#include "nvim/types_defs.h"  // IWYU pragma: keep (TriState, used by mcursor.h.generated.h)

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void mc_init(void);
DLLEXPORT size_t mc_count(void);
DLLEXPORT size_t mc_showcmd(char *buf, size_t size);
DLLEXPORT bool mc_replaying(void);
DLLEXPORT void mc_clock_edge(bool map_edit, bool map_moved, bool follow);
DLLEXPORT void mc_ins_join(void);
DLLEXPORT bool mc_ins_replay_can_join(void);
DLLEXPORT void mc_ins_cascade_start(bool cascade, CmdOrigin origin, uint64_t root_frame,
                                    const char *vsel);
DLLEXPORT void mc_ins_cascade_restart(void);
DLLEXPORT bool mc_ins_replaying(void);
DLLEXPORT bool mc_ins_cascading(void);
DLLEXPORT bool mc_ins_cascaded(uint64_t root_frame);
DLLEXPORT void mc_ins_cascade(void);
DLLEXPORT void mc_vsel_clear(void);
DLLEXPORT void mc_vsel_refresh(void);
DLLEXPORT void mc_undo_time(void);
DLLEXPORT bool mc_ins_commit(void);
DLLEXPORT bool mc_buf_has_cursors(buf_T *buf);
DLLEXPORT bool mc_following(void);
DLLEXPORT uint32_t mc_mark_at(buf_T *buf, pos_T pos, uint32_t skip);
DLLEXPORT void mc_zq_start(cmdarg_T *cap);
DLLEXPORT void mc_zq(oparg_T *oap, cmdarg_T *cap, pos_T origin);
DLLEXPORT void mc_counter(long count1);
DLLEXPORT void mc_follow_set(TriState on);
DLLEXPORT void mc_toggle(buf_T *buf, pos_T pos, bool end_follow, TriState on);
DLLEXPORT Context *mc_add(buf_T *buf, pos_T pos);
DLLEXPORT void mc_on_extmark_set(buf_T *buf, uint32_t ns_id, uint32_t id, pos_T pos);
DLLEXPORT void mc_buf_clear(buf_T *buf);
DLLEXPORT void mc_buf_free(buf_T *buf);
DLLEXPORT void mc_ns_clearing(buf_T *buf, uint32_t ns_id);
DLLEXPORT void mc_ns_cleared(buf_T *buf, uint32_t ns_id);
DLLEXPORT void mc_free_all(void);
