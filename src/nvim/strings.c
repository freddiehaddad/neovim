#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <string.h>

#include "auto/config.h"
#include "nvim/ascii_defs.h"
#include "nvim/eval/typval_defs.h"
#include "nvim/strings.h"
#include "nvim/types_defs.h"

#include "strings.c.generated.h"

typedef struct {
  va_list ap_start;
  va_list ap;
} VaState;

int vim_vsnprintf_typval_rs(char *str, size_t str_m, const char *fmt, void *va_ptr,
                            typval_T *const tvs);
long long va_state_llong(void *va_ptr);  // NOLINT(runtime/int)
unsigned long va_state_ulong(void *va_ptr);  // NOLINT(runtime/int)
unsigned long long va_state_ullong(void *va_ptr);  // NOLINT(runtime/int)

void va_state_reset(void *va_ptr)
{
  VaState *va = va_ptr;
  va_end(va->ap);
  va_copy(va->ap, va->ap_start);
}

int va_state_int(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, int);
}

long va_state_long(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, long);  // NOLINT(runtime/int)
}

long long va_state_llong(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, long long);  // NOLINT(runtime/int)
}

ptrdiff_t va_state_ptrdiff(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, ptrdiff_t);
}

unsigned va_state_uint(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, unsigned);
}

unsigned long va_state_ulong(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, unsigned long);  // NOLINT(runtime/int)
}

unsigned long long va_state_ullong(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, unsigned long long);  // NOLINT(runtime/int)
}

size_t va_state_size(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, size_t);
}

const char *va_state_str(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, const char *);
}

void *va_state_ptr(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, void *);
}

double va_state_double(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, double);
}

void va_report_null_string(void)
{
#ifdef ENABLE_ASAN_UBSAN
  __sanitizer_print_stack_trace();
  __sanitizer_report_error_summary("SUMMARY: NULL argument passed to %s");
#endif
}

/// Write formatted value to the string
///
/// @param[out]  str  String to write to.
/// @param[in]  str_m  String length.
/// @param[in]  fmt  String format.
///
/// @return Number of bytes excluding NUL byte that would be written to the
///         string if str_m was greater or equal to the return value.
int vim_snprintf(char *str, size_t str_m, const char *fmt, ...)
  FUNC_ATTR_PRINTF(3, 4)
{
  va_list ap;
  va_start(ap, fmt);
  VaState va;
  va_copy(va.ap_start, ap);
  va_copy(va.ap, ap);
  const int str_l = vim_vsnprintf_typval_rs(str, str_m, fmt, &va, NULL);
  va_end(va.ap);
  va_end(va.ap_start);
  va_end(ap);
  return str_l;
}
