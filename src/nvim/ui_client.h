#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "nvim/grid_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/types_defs.h"
#include "nvim/ui_defs.h"  // IWYU pragma: keep

// Temporary buffer for converting a single grid_line event
EXTERN size_t grid_line_buf_size INIT( = 0);
EXTERN schar_T *grid_line_buf_char INIT( = NULL);
EXTERN sattr_T *grid_line_buf_attr INIT( = NULL);

// Client-side UI channel. Zero during early startup or if not a (--remote-ui) UI client.
EXTERN uint64_t ui_client_channel_id INIT( = 0);

/// `status` argument of the last "error_exit" UI event, or -1 if none has been seen.
/// NOTE: This assumes "error_exit" never has a negative `status` argument.
EXTERN int ui_client_error_exit INIT( = -1);

/// Server exit code.
EXTERN int ui_client_exit_status INIT( = 0);

/// Whether ui client has sent nvim_ui_attach yet
EXTERN bool ui_client_attached INIT( = false);

/// The ui client should forward its stdin to the nvim process
/// by convention, this uses fd=3 (next free number after stdio)
EXTERN bool ui_client_forward_stdin INIT( = false);

#define UI_CLIENT_STDIN_FD 3

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

DLLEXPORT uint64_t ui_client_start_server(const char *exepath, size_t argc, char **argv);
DLLEXPORT void ui_client_attach(int width, int height, char *term, bool rgb);
DLLEXPORT void ui_client_detach(void);
DLLEXPORT void ui_client_run(void) FUNC_ATTR_NORETURN;
DLLEXPORT void ui_client_stop(void);
DLLEXPORT void ui_client_set_size(int width, int height);
DLLEXPORT Object handle_ui_client_redraw(uint64_t channel_id, Array args, Arena *arena,
                                         Error *error);
DLLEXPORT void ui_client_event_grid_resize(Array args);
DLLEXPORT void ui_client_event_grid_line(Array args) FUNC_ATTR_NORETURN;
DLLEXPORT void ui_client_event_raw_line(GridLineEvent *g);
DLLEXPORT void ui_client_event_connect(Array args);
DLLEXPORT void ui_client_event_restart(Array args);
DLLEXPORT void ui_client_event__set_restart_on_crash_exit(Array args);
DLLEXPORT void ui_client_attach_to_restarted_server(bool error_restart);
DLLEXPORT void ui_client_event_error_exit(Array args);
DLLEXPORT void ui_client_free_all_mem(void);
DLLEXPORT HlAttrs ui_client_dict2hlattrs(Dict d, bool rgb);
DLLEXPORT UIClientHandler ui_client_get_redraw_handler(const char *name, size_t name_len,
                                                       Error *error);

#include "nvim/func_attr.h"
