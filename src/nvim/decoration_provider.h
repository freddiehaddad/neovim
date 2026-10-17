#pragma once

#include <stdbool.h>

#include "nvim/decoration_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep

DLLEXPORT void decor_providers_invoke_spell(win_T *wp, int start_row, int start_col, int end_row,
                                            int end_col);
DLLEXPORT bool decor_providers_invoke_conceal_line(win_T *wp, int row);
DLLEXPORT void decor_providers_start(void);
DLLEXPORT void decor_providers_invoke_win(win_T *wp);
DLLEXPORT void decor_providers_invoke_line(win_T *wp, int row);
DLLEXPORT void decor_providers_invoke_range(win_T *wp, int start_row, int start_col, int end_row,
                                            int end_col);
DLLEXPORT void decor_providers_invoke_buf(buf_T *buf);
DLLEXPORT void decor_providers_invoke_end(void);
DLLEXPORT void decor_provider_invalidate_hl(void);
DLLEXPORT DecorProvider *get_decor_provider(NS ns_id, bool force);
DLLEXPORT void decor_provider_clear(DecorProvider *p);
DLLEXPORT void decor_free_all_mem(void);
