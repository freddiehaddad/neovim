#include <assert.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

#include "auto/config.h"
#include "nvim/api/private/defs.h"
#include "nvim/api/private/helpers.h"
#include "nvim/ascii_defs.h"
#include "nvim/eval/typval_defs.h"
#include "nvim/macros_defs.h"
#include "nvim/memory.h"
#include "nvim/memory_defs.h"
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

/// Append a formatted value to the string
///
/// @see vim_vsnprintf_typval().
int vim_snprintf_add(char *str, size_t str_m, const char *fmt, ...)
  FUNC_ATTR_PRINTF(3, 4)
{
  const size_t len = strlen(str);
  size_t space;

  if (str_m <= len) {
    space = 0;
  } else {
    space = str_m - len;
  }
  va_list ap;
  va_start(ap, fmt);
  const int str_l = vim_vsnprintf(str + len, space, fmt, ap);
  va_end(ap);
  return str_l;
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
  const int str_l = vim_vsnprintf(str, str_m, fmt, ap);
  va_end(ap);
  return str_l;
}

/// Like vim_snprintf() except the return value can be safely used to increment a
/// buffer length.
/// Normal `snprintf()` (and `vim_snprintf()`) returns the number of bytes that
/// would have been copied if the destination buffer was large enough.
/// This means that you cannot rely on it's return value for the destination
/// length because the destination may be shorter than the source. This function
/// guarantees the returned length will never be greater than the destination length.
size_t vim_snprintf_safelen(char *str, size_t str_m, const char *fmt, ...)
{
  va_list ap;
  int str_l;

  if (str_m == 0) {
    return 0;
  }

  va_start(ap, fmt);
  str_l = vim_vsnprintf_typval(str, str_m, fmt, ap, NULL);
  va_end(ap);

  if (str_l < 0) {
    *str = NUL;
    return 0;
  }
  return ((size_t)str_l >= str_m) ? str_m - 1 : (size_t)str_l;
}

int vim_vsnprintf(char *str, size_t str_m, const char *fmt, va_list ap)
{
  return vim_vsnprintf_typval(str, str_m, fmt, ap, NULL);
}

int vim_vsnprintf_typval(char *str, size_t str_m, const char *fmt, va_list ap_start,
                         typval_T *const tvs)
{
  VaState va;
  va_copy(va.ap_start, ap_start);
  va_copy(va.ap, ap_start);
  int ret = vim_vsnprintf_typval_rs(str, str_m, fmt, &va, tvs);
  va_end(va.ap);
  va_end(va.ap_start);
  return ret;
}

int kv_do_printf(StringBuilder *str, const char *fmt, ...)
  FUNC_ATTR_PRINTF(2, 3)
{
  size_t remaining = str->capacity - str->size;

  va_list ap;
  va_start(ap, fmt);
  int printed = vsnprintf(str->items ? str->items + str->size : NULL, remaining, fmt, ap);
  va_end(ap);

  if (printed < 0) {
    return -1;
  }

  // printed string didn't fit, resize and try again
  if ((size_t)printed >= remaining) {
    kv_ensure_space(*str, (size_t)printed + 1);  // include space for NUL terminator at the end
    assert(str->items != NULL);
    va_start(ap, fmt);
    printed = vsnprintf(str->items + str->size, str->capacity - str->size, fmt, ap);
    va_end(ap);
    if (printed < 0) {
      return -1;
    }
  }

  str->size += (size_t)printed;
  return printed;
}

String arena_printf(Arena *arena, const char *fmt, ...)
  FUNC_ATTR_PRINTF(2, 3)
{
  size_t remaining = 0;
  char *buf = NULL;
  if (arena) {
    if (!arena->cur_blk) {
      arena_alloc_block(arena);
    }

    // happy case, we can fit the printed string in the rest of the current
    // block (one pass):
    remaining = arena->size - arena->pos;
    buf = arena->cur_blk + arena->pos;
  }

  va_list ap;
  va_start(ap, fmt);
  int printed = vsnprintf(buf, remaining, fmt, ap);
  va_end(ap);

  if (printed < 0) {
    return (String)STRING_INIT;
  }

  // printed string didn't fit, allocate and try again
  if ((size_t)printed >= remaining) {
    buf = arena_alloc(arena, (size_t)printed + 1, false);
    va_start(ap, fmt);
    printed = vsnprintf(buf, (size_t)printed + 1, fmt, ap);
    va_end(ap);
    if (printed < 0) {
      return (String)STRING_INIT;
    }
  } else {
    arena->pos += (size_t)printed + 1;
  }

  return cbuf_as_string(buf, (size_t)printed);
}
