#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

typedef void (*terminal_read_pause_cb)(bool pause, void *data);
typedef void (*terminal_write_cb)(const char *buffer, size_t size, void *data);
typedef void (*terminal_resize_cb)(uint16_t width, uint16_t height, void *data);
typedef void (*terminal_resume_cb)(void *data);
typedef void (*terminal_close_cb)(void *data);

typedef struct {
  void *data;  // PTY process channel
  uint16_t width, height;
  terminal_read_pause_cb read_pause_cb;
  terminal_write_cb write_cb;
  terminal_resize_cb resize_cb;
  terminal_resume_cb resume_cb;
  terminal_close_cb close_cb;
  bool force_crlf;
} TerminalOptions;

#include "nvim/func_attr.h"

DLLEXPORT void terminal_init(void);
DLLEXPORT void terminal_teardown(void);
DLLEXPORT Terminal *terminal_alloc(buf_T *buf, TerminalOptions opts) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void terminal_open(Terminal **termpp, buf_T *buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void terminal_close(Terminal **termpp, int status) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void terminal_set_state(Terminal *term, bool suspended) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void terminal_check_size(Terminal *term) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool terminal_enter(void);
DLLEXPORT void terminal_destroy(Terminal **termpp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void terminal_set_streamed_paste(Terminal *term, bool streamed) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void terminal_paste(int count, String *y_array, size_t y_size);
DLLEXPORT void terminal_receive(Terminal *term, const char *data, size_t len);
DLLEXPORT void terminal_get_line_attributes(Terminal *term, win_T *wp, int linenr, int *term_attrs);
DLLEXPORT Buffer terminal_buf(const Terminal *term) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool terminal_running(const Terminal *term) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool terminal_suspended(const Terminal *term) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void terminal_notify_theme(Terminal *term, bool dark) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void terminal_check_refresh(void);
DLLEXPORT void on_scrollback_option_changed(Terminal *term);
