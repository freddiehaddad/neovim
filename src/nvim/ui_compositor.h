#pragma once

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/grid_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep
#include "nvim/ui_defs.h"  // IWYU pragma: keep

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

DLLEXPORT void ui_comp_init(void);
DLLEXPORT void ui_comp_free_all_mem(void);
DLLEXPORT void ui_comp_syn_init(void);
DLLEXPORT void ui_comp_attach(RemoteUI *ui);
DLLEXPORT void ui_comp_detach(RemoteUI *ui);
DLLEXPORT bool ui_comp_should_draw(void);
DLLEXPORT void ui_comp_layers_adjust(size_t layer_idx, bool raise);
DLLEXPORT bool ui_comp_put_grid(ScreenGrid *grid, int row, int col, int height, int width,
                                bool valid, bool on_top);
DLLEXPORT void ui_comp_remove_grid(ScreenGrid *grid);
DLLEXPORT bool ui_comp_set_grid(handle_T handle);
DLLEXPORT void ui_comp_raise_grid(ScreenGrid *grid, size_t new_index);
DLLEXPORT void ui_comp_grid_cursor_goto(Integer grid_handle, Integer r, Integer c);
DLLEXPORT ScreenGrid *ui_comp_mouse_focus(int row, int col);
DLLEXPORT ScreenGrid *ui_comp_get_grid_at_coord(int row, int col);
DLLEXPORT void ui_comp_compose_grid(ScreenGrid *grid);
DLLEXPORT void ui_comp_raw_line(Integer grid, Integer row, Integer startcol, Integer endcol,
                                Integer clearcol, Integer clearattr, LineFlags flags,
                                const schar_T *chunk, const sattr_T *attrs);
DLLEXPORT bool ui_comp_set_screen_valid(bool valid);
DLLEXPORT void ui_comp_msg_set_pos(Integer grid, Integer row, Boolean scrolled, String sep_char,
                                   Integer zindex, Integer compindex);
DLLEXPORT void ui_comp_grid_scroll(Integer grid, Integer top, Integer bot, Integer left,
                                   Integer right, Integer rows, Integer cols);
DLLEXPORT void ui_comp_grid_resize(Integer grid, Integer width, Integer height);

#include "nvim/func_attr.h"
