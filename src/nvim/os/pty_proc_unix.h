#pragma once
// IWYU pragma: private, include "nvim/os/pty_proc.h"

#include <stdint.h>
#include <sys/ioctl.h>

#include "nvim/event/defs.h"
#include "nvim/event/proc.h"

typedef struct {
  Proc proc;
  uint16_t width, height;
  struct winsize winsize;
  int tty_fd;
} PtyProc;

static inline PtyProc pty_proc_init(Loop *loop, void *data)
{
  PtyProc rv = { 0 };
  rv.proc = proc_init(loop, kProcTypePty, data);
  rv.width = 80;
  rv.height = 24;
  rv.tty_fd = -1;
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
DLLEXPORT void pty_proc_teardown(Loop *loop);

#include "nvim/func_attr.h"
