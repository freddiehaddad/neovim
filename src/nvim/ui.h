#pragma once

#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/event/defs.h"
#include "nvim/grid_defs.h"  // IWYU pragma: keep
#include "nvim/highlight_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
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

DLLEXPORT void ui_init(void);
DLLEXPORT void ui_free_all_mem(void);
DLLEXPORT bool ui_rgb_attached(void);
DLLEXPORT bool ui_gui_attached(void);
DLLEXPORT bool ui_override(void);
DLLEXPORT size_t ui_active(void);
DLLEXPORT void ui_refresh(void);
DLLEXPORT int ui_pum_get_height(void);
DLLEXPORT bool ui_pum_get_pos(double *pwidth, double *pheight, double *prow, double *pcol);
DLLEXPORT void ui_schedule_refresh(void);
DLLEXPORT void ui_default_colors_set(void);
DLLEXPORT void ui_busy_start(void);
DLLEXPORT void ui_busy_stop(void);
DLLEXPORT void vim_beep(unsigned val);
DLLEXPORT void do_autocmd_uienter_all(void);
DLLEXPORT bool ui_can_attach_more(void);
DLLEXPORT void ui_attach_impl(RemoteUI *ui, uint64_t chanid);
DLLEXPORT void ui_detach_impl(RemoteUI *ui, uint64_t chanid);
DLLEXPORT void ui_set_ext_option(RemoteUI *ui, UIExtension ext, bool active);
DLLEXPORT void ui_line(ScreenGrid *grid, int row, bool invalid_row, int startcol, int endcol,
                       int clearcol, int clearattr, bool wrap);
DLLEXPORT void ui_cursor_goto(int new_row, int new_col);
DLLEXPORT void ui_grid_cursor_goto(handle_T grid_handle, int new_row, int new_col);
DLLEXPORT void ui_check_cursor_grid(handle_T grid_handle);
DLLEXPORT void ui_mode_info_set(void);
DLLEXPORT int ui_current_row(void);
DLLEXPORT int ui_current_col(void);
DLLEXPORT void ui_flush(void);
DLLEXPORT void ui_check_mouse(void);
DLLEXPORT bool ui_mouse_has(int mode);
DLLEXPORT void ui_cursor_shape_no_check_conceal(void);
DLLEXPORT void ui_cursor_shape(void);
DLLEXPORT bool ui_has(UIExtension ext);
DLLEXPORT Array ui_array(Arena *arena);
DLLEXPORT void ui_grid_resize(handle_T grid_handle, int width, int height, Error *err);
DLLEXPORT void ui_call_event(char *name, Array args);
DLLEXPORT void ui_add_cb(uint32_t ns_id, LuaRef cb, bool *ext_widgets);
DLLEXPORT void ui_remove_cb(uint32_t ns_id, bool checkerr);
DLLEXPORT void ui_log(const char *funname);

// uncrustify:off
#include "ui_events_call.h.generated.h"
EXTERN Array noargs INIT(= ARRAY_DICT_INIT);
// uncrustify:on

// vim.ui_attach() namespace of currently executed callback.
EXTERN uint32_t ui_event_ns_id INIT( = 0);
EXTERN MultiQueue *resize_events INIT( = NULL);
EXTERN bool ui_refresh_cmdheight INIT( = true);
