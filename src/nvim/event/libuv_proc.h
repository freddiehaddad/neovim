#pragma once

#include <uv.h>

#include "nvim/event/defs.h"
#include "nvim/event/proc.h"

typedef struct {
  Proc proc;
  uv_process_t uv;
  uv_process_options_t uvopts;
  uv_stdio_container_t uvstdio[4];
} LibuvProc;

static inline LibuvProc libuv_proc_init(Loop *loop, void *data)
{
  LibuvProc rv = {
    .proc = proc_init(loop, kProcTypeUv, data)
  };
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

DLLEXPORT int libuv_proc_spawn(LibuvProc *uvproc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void libuv_proc_close(LibuvProc *uvproc) FUNC_ATTR_NONNULL_ARG(1);

#include "nvim/func_attr.h"
