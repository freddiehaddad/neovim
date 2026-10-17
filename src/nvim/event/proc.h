#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "nvim/event/defs.h"  // IWYU pragma: keep
#include "nvim/os/os_defs.h"
#include "nvim/types_defs.h"

static inline Proc proc_init(Loop *loop, ProcType type, void *data)
{
  return (Proc) {
    .type = type,
    .data = data,
    .loop = loop,
    .events = NULL,
    .pid = 0,
    .status = -1,
    .refcount = 0,
    .stopped_time = 0,
    .cwd = NULL,
    .argv = NULL,
    .exepath = NULL,
    .in = { .closed = false },
    .out = { .s.closed = false, .s.fd = STDOUT_FILENO },
    .err = { .s.closed = false, .s.fd = STDERR_FILENO },
    .cb = NULL,
    .state_cb = NULL,
    .closed = false,
    .internal_close_cb = NULL,
    .internal_exit_cb = NULL,
    .detach = false,
    .fwd_err = false,
    .stdio_noinherit = false,
  };
}

/// Get the path to the executable of the process.
static inline const char *proc_get_exepath(Proc *proc)
{
  return proc->exepath != NULL ? proc->exepath : proc->argv[0];
}

static inline bool proc_is_stopped(Proc *proc)
{
  bool exited = (proc->status >= 0);
  return exited || (proc->stopped_time != 0);
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

DLLEXPORT void __gcov_flush(void);
DLLEXPORT int proc_spawn(Proc *proc, bool in, bool out, bool err) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void proc_teardown(Loop *loop) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void proc_close_streams(Proc *proc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int proc_wait(Proc *proc, int ms, MultiQueue *events) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void proc_stop(Proc *proc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void proc_free(Proc *proc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void exit_on_closed_chan(int status);

#include "nvim/func_attr.h"
