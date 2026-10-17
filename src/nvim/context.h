#pragma once

#include <stddef.h>  // IWYU pragma: keep
#include <stdint.h>

#include "nvim/context_defs.h"  // IWYU pragma: export
#include "nvim/macros_defs.h"

/// Pool of temporary scratch windows (fka "autocmd windows"), for ctx_switch().
EXTERN kvec_t(CtxWin) ctx_win_vec INIT( = KV_INITIAL_VALUE);
#define ctx_win (ctx_win_vec.items)
#define CTX_WIN_COUNT ((int)ctx_win_vec.size)

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

DLLEXPORT void ctx_free(Context *ctx) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void ctx_save(Context *ctx, const CtxStateFlags flags) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void ctx_load(Context *ctx, const CtxStateFlags flags, const CtxLoadFlags loadflags)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT Dict ctx_to_dict(Context *ctx, Arena *arena) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT CtxStateFlags ctx_from_dict(Dict dict, Context *ctx, Error *err) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void ctx_did_chdir(void);
DLLEXPORT bool is_ctx_win(win_T *win);
DLLEXPORT win_T *ctx_saved_curwin(void);
DLLEXPORT bool ctx_switch(CtxSwitch *cs, win_T *wp, tabpage_T *tp, buf_T *buf,
                          CtxSwitchFlags flags);
DLLEXPORT void ctx_restore(CtxSwitch *cs);

#include "nvim/func_attr.h"
