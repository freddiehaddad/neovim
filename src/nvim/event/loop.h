#pragma once

#include <stdbool.h>
#include <uv.h>

#include "klib/kvec.h"
#include "nvim/event/defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

struct loop {
  uv_loop_t uv;
  MultiQueue *events;
  MultiQueue *thread_events;
  // Immediate events.
  // - "Processed after exiting `uv_run()` (to avoid recursion), but before returning from
  //   `loop_poll_events()`." 502aee690c98
  // - Practical consequence (for `main_loop`):
  //    - these are processed by `state_enter()..input_get()` whereas "regular" events
  //      (`main_loop.events`) are processed by `state_enter()..VimState.execute()`
  //    - `state_enter()..input_get()` can be "too early" if you want the event to trigger UI
  //      updates and other user-activity-related side-effects.
  MultiQueue *fast_events;

  // used by process/job-control subsystem
  kvec_t(Proc *) children;
  uv_signal_t children_watcher;
  uv_timer_t children_kill_timer;

  // generic timer, used by loop_poll_events()
  uv_timer_t poll_timer;

  uv_timer_t exit_delay_timer;

  uv_async_t async;
  uv_mutex_t mutex;
  int recursive;
  bool closing;  ///< Set to true if loop_close() has been called
};

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

DLLEXPORT void loop_init(Loop *loop, void *data);
DLLEXPORT bool loop_poll_events(Loop *loop, int64_t ms);
DLLEXPORT void loop_schedule_fast(Loop *loop, Event event);
DLLEXPORT void loop_schedule_deferred(Loop *loop, Event event);
DLLEXPORT void loop_on_put(MultiQueue *queue, void *data);
DLLEXPORT bool loop_close(Loop *loop, bool wait);
DLLEXPORT void loop_purge(Loop *loop);
DLLEXPORT size_t loop_size(Loop *loop);

#include "nvim/func_attr.h"
