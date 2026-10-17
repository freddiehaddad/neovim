#pragma once
// IWYU pragma: private, include "nvim/os/pty_proc.h"

#include <uv.h>

#include "nvim/event/proc.h"
#include "nvim/lib/queue_defs.h"
#include "nvim/os/pty_conpty_win.h"

typedef struct pty_process {
  Proc proc;
  uint16_t width, height;
  conpty_t *conpty;
  HANDLE finish_wait;
  HANDLE proc_handle;
} PtyProc;

// Structure used by build_cmd_line()
typedef struct arg_node {
  char *arg;  // pointer to argument.
  QUEUE node;  // QUEUE structure.
} ArgNode;

static inline PtyProc pty_proc_init(Loop *loop, void *data)
{
  PtyProc rv;
  rv.proc = proc_init(loop, kProcTypePty, data);
  rv.width = 80;
  rv.height = 24;
  rv.conpty = NULL;
  rv.finish_wait = NULL;
  rv.proc_handle = NULL;
  return rv;
}

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

DLLEXPORT int pty_proc_spawn(PtyProc *ptyproc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT const char *pty_proc_tty_name(PtyProc *ptyproc);
DLLEXPORT void pty_proc_resize(PtyProc *ptyproc, uint16_t width,
                               uint16_t height) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void pty_proc_resume(PtyProc *ptyproc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void pty_proc_flush_master(PtyProc *ptyproc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void pty_proc_close(PtyProc *ptyproc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void pty_proc_close_master(PtyProc *ptyproc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void pty_proc_teardown(Loop *loop) FUNC_ATTR_NONNULL_ALL;

#include "nvim/func_attr.h"
