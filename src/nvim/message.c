// message.c: C variadic wrappers for message.rs

#include <stdarg.h>
#include <stdbool.h>
#include <stdlib.h>

#include "nvim/globals.h"
#include "nvim/highlight_defs.h"
#include "nvim/message.h"
#include "nvim/strings.h"
#include "nvim/ui.h"

#include "message.c.generated.h"

/// Shows a printf-style message with highlight id.
int smsg(int hl_id, const char *s, ...)
  FUNC_ATTR_PRINTF(2, 3)
{
  va_list arglist;

  va_start(arglist, s);
  vim_vsnprintf(IObuff, IOSIZE, s, arglist);
  va_end(arglist);
  return msg(IObuff, hl_id);
}

int smsg_keep(int hl_id, const char *s, ...)
  FUNC_ATTR_PRINTF(2, 3)
{
  va_list arglist;

  va_start(arglist, s);
  vim_vsnprintf(IObuff, IOSIZE, s, arglist);
  va_end(arglist);
  return msg_keep(IObuff, hl_id, true, false);
}

/// Print an error message with unknown number of arguments
bool semsg(const char *const fmt, ...)
  FUNC_ATTR_PRINTF(1, 2)
{
  va_list ap;
  va_start(ap, fmt);
  bool ret = semsgv(fmt, ap);
  va_end(ap);

  return ret;
}

#define MULTILINE_BUFSIZE 8192

bool semsg_multiline(const char *kind, const char *const fmt, ...)
{
  static char errbuf[MULTILINE_BUFSIZE];
  if (emsg_not_now()) {
    return true;
  }

  va_list ap;
  va_start(ap, fmt);
  vim_vsnprintf(errbuf, sizeof(errbuf), fmt, ap);
  va_end(ap);

  return emsg_multiline(errbuf, kind, HLF_E, true);
}

/// Print an error message with unknown number of arguments
static bool semsgv(const char *fmt, va_list ap)
{
  static char errbuf[IOSIZE];
  if (emsg_not_now()) {
    return true;
  }

  vim_vsnprintf(errbuf, sizeof(errbuf), fmt, ap);

  return emsg(errbuf);
}

/// Same as semsg(...) but abort on error when ABORT_ON_INTERNAL_ERROR is
/// defined. It is used for internal errors only, so that they can be
/// detected when fuzzing vim.
void siemsg(const char *s, ...)
{
  if (emsg_not_now()) {
    return;
  }

  va_list ap;
  va_start(ap, s);
  semsgv(s, ap);
  va_end(ap);
#ifdef ABORT_ON_INTERNAL_ERROR
  msg_putchar('\n');  // avoid overwriting the error message
  ui_flush();
  abort();
#endif
}

void msg_schedule_semsg(const char *const fmt, ...)
  FUNC_ATTR_PRINTF(1, 2)
{
  va_list ap;
  va_start(ap, fmt);
  vim_vsnprintf(IObuff, IOSIZE, fmt, ap);
  va_end(ap);

  msg_schedule_semsg_cstr(IObuff);
}

void msg_schedule_semsg_multiline(const char *const fmt, ...)
{
  va_list ap;
  va_start(ap, fmt);
  vim_vsnprintf(IObuff, IOSIZE, fmt, ap);
  va_end(ap);

  msg_schedule_semsg_multiline_cstr(IObuff);
}

void swmsg(bool hl, const char *const fmt, ...)
  FUNC_ATTR_PRINTF(2, 3)
{
  va_list args;

  va_start(args, fmt);
  vim_vsnprintf(IObuff, IOSIZE, fmt, args);
  va_end(args);

  give_warning(IObuff, hl, true);
}
