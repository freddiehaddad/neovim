// ex_docmd.c: C variadic shim for Ex command line execution (`src/nvim/ex_docmd.rs`).

#include <stdarg.h>

#include "nvim/ex_docmd.h"
#include "nvim/gettext_defs.h"
#include "nvim/globals.h"
#include "nvim/strings.h"

#include "ex_docmd.c.generated.h"

static char ex_error_buf[MSG_BUF_LEN];

/// @return an error message with argument included.
/// Uses a static buffer, only the last error will be kept.
/// "msg" will be translated, caller should use N_().
char *ex_errmsg(const char *const msg, ...)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PRINTF(1, 2)
{
  va_list ap;
  va_start(ap, msg);
  vim_vsnprintf(ex_error_buf, MSG_BUF_LEN, _(msg), ap);
  va_end(ap);
  return ex_error_buf;
}
