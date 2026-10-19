//
// Log module C variadic shim (`src/nvim/log.rs`).
//

#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>

#include "auto/config.h"
#include "nvim/log.h"
#include "nvim/macros_defs.h"
#include "nvim/memory.h"

#ifdef HAVE_EXECINFO_BACKTRACE
# include <execinfo.h>
#endif

#include "log.c.generated.h"

FILE *log_get_stderr(void)
{
  return stderr;
}

bool log_is_std_stream(FILE *fp)
{
  return fp == stderr || fp == stdout;
}

/// Logs a message to $NVIM_LOG_FILE.
///
/// @param log_level  Log level (see log.h)
/// @param context    Description of a shared context or subsystem
/// @param func_name  Function name, or NULL
/// @param line_num   Source line number, or -1
/// @param eol        Append linefeed "\n"
/// @param fmt        printf-style format string
///
/// @return true if log was emitted normally, false if failed or recursive
bool logmsg(int log_level, const char *context, const char *func_name, int line_num, bool eol,
            const char *fmt, ...)
  FUNC_ATTR_PRINTF(6, 7)
{
  if (!log_should_log(log_level)) {
    return false;
  }

  char stack_buf[1024];
  va_list args;
  va_start(args, fmt);
  va_list args_copy;
  va_copy(args_copy, args);
  int len = vsnprintf(stack_buf, sizeof(stack_buf), fmt, args);
  va_end(args);

  char *heap_buf = NULL;
  const char *msg = stack_buf;
  if (len >= (int)sizeof(stack_buf)) {
    heap_buf = xmalloc((size_t)len + 1);
    vsnprintf(heap_buf, (size_t)len + 1, fmt, args_copy);
    msg = heap_buf;
  }
  va_end(args_copy);

  bool ret = logmsg_str(log_level, context, func_name, line_num, eol, msg);
  xfree(heap_buf);
  return ret;
}

#ifdef HAVE_EXECINFO_BACKTRACE
void log_callstack_to_file(FILE *log_file, const char *const func_name, const int line_num)
{
  void *trace[100];
  int trace_size = backtrace(trace, ARRAY_SIZE(trace));
  log_callstack_to_file_trace(log_file, func_name, line_num, trace, trace_size);
}

void log_callstack(const char *const func_name, const int line_num)
{
  log_lock();
  FILE *log_file = open_log_file();
  log_callstack_to_file(log_file, func_name, line_num);
  log_unlock();
}
#endif
