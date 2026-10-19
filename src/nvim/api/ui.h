#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/grid_defs.h"  // IWYU pragma: keep
#include "nvim/highlight_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep
#include "nvim/ui_defs.h"  // IWYU pragma: keep

/// Keep in sync with UIExtension in ui_defs.h
EXTERN const char *ui_ext_names[] INIT( = {
  "ext_cmdline",
  "ext_popupmenu",
  "ext_tabline",
  "ext_wildmenu",
  "ext_messages",
  "ext_linegrid",
  "ext_multigrid",
  "ext_hlstate",
  "ext_termcolors",
  "_debug_float",
});

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

DLLEXPORT void remote_ui_disconnect(uint64_t channel_id, Error *err, bool send_error_exit);
DLLEXPORT void ui_detach_channel(uint64_t chan, Error *err);
DLLEXPORT size_t ui_detach_others(uint64_t keep_chan);
DLLEXPORT void remote_ui_free_all_mem(void);
DLLEXPORT void remote_ui_wait_for_attach(void);
DLLEXPORT void nvim_ui_attach(uint64_t channel_id, Integer width, Integer height, Dict options,
                              Error *err) FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
DLLEXPORT void ui_attach(uint64_t channel_id, Integer width, Integer height, Boolean enable_rgb,
                         Error *err) FUNC_API_DEPRECATED_SINCE(1);
DLLEXPORT void nvim_ui_set_focus(uint64_t channel_id, Boolean gained, Error *error)
  FUNC_API_SINCE(11) FUNC_API_REMOTE_ONLY;
DLLEXPORT void nvim_ui_detach(uint64_t channel_id, Error *err)
  FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
DLLEXPORT void remote_ui_connect(uint64_t channel_id, char *server_addr, Error *err);
DLLEXPORT void remote_ui_restart(String listen_addr);
DLLEXPORT void nvim_ui_try_resize(uint64_t channel_id, Integer width, Integer height, Error *err)
  FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
DLLEXPORT void nvim_ui_set_option(uint64_t channel_id, String name, Object value, Error *error)
  FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
DLLEXPORT void nvim_ui_try_resize_grid(uint64_t channel_id, Integer grid, Integer width,
                                       Integer height, Error *err)
  FUNC_API_SINCE(6) FUNC_API_REMOTE_ONLY;
DLLEXPORT void nvim_ui_pum_set_height(uint64_t channel_id, Integer height, Error *err)
  FUNC_API_SINCE(6) FUNC_API_REMOTE_ONLY;
DLLEXPORT void nvim_ui_pum_set_bounds(uint64_t channel_id, Float width, Float height, Float row,
                                      Float col, Error *err)
  FUNC_API_SINCE(7) FUNC_API_REMOTE_ONLY;
DLLEXPORT void remote_ui_grid_clear(RemoteUI *ui, Integer grid);
DLLEXPORT void remote_ui_grid_resize(RemoteUI *ui, Integer grid, Integer width, Integer height);
DLLEXPORT void remote_ui_grid_scroll(RemoteUI *ui, Integer grid, Integer top, Integer bot,
                                     Integer left, Integer right, Integer rows, Integer cols);
DLLEXPORT void remote_ui_default_colors_set(RemoteUI *ui, Integer rgb_fg, Integer rgb_bg,
                                            Integer rgb_sp, Integer cterm_fg, Integer cterm_bg);
DLLEXPORT void remote_ui_hl_attr_define(RemoteUI *ui, Integer id, HlAttrs rgb_attrs,
                                        HlAttrs cterm_attrs, Array info);
DLLEXPORT void remote_ui_highlight_set(RemoteUI *ui, int id);
DLLEXPORT void remote_ui_grid_cursor_goto(RemoteUI *ui, Integer grid, Integer row, Integer col);
DLLEXPORT void remote_ui_cursor_goto(RemoteUI *ui, Integer row, Integer col);
DLLEXPORT void remote_ui_put(RemoteUI *ui, const char *cell);
DLLEXPORT void remote_ui_raw_line(RemoteUI *ui, Integer grid, Integer row, Integer startcol,
                                  Integer endcol, Integer clearcol, Integer clearattr,
                                  LineFlags flags, const schar_T *chunk, const sattr_T *attrs);
DLLEXPORT void remote_ui_flush(RemoteUI *ui);
DLLEXPORT void remote_ui_ui_send(RemoteUI *ui, String content);
DLLEXPORT void remote_ui_flush_pending_data(RemoteUI *ui);
DLLEXPORT void remote_ui_event(RemoteUI *ui, char *name, Array args);
DLLEXPORT void nvim_ui_send(uint64_t channel_id, String content, Error *err) FUNC_API_SINCE(14);

#include "api/ui.h.generated.h"
#include "ui_events_remote.h.generated.h"
