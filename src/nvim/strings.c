#include <assert.h>
#include <inttypes.h>
#include <math.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "auto/config.h"
#include "nvim/api/private/defs.h"
#include "nvim/api/private/helpers.h"
#include "nvim/ascii_defs.h"
#include "nvim/assert_defs.h"
#include "nvim/charset.h"
#include "nvim/errors.h"
#include "nvim/eval/encode.h"
#include "nvim/eval/typval.h"
#include "nvim/eval/typval_defs.h"
#include "nvim/ex_docmd.h"
#include "nvim/garray.h"
#include "nvim/garray_defs.h"
#include "nvim/gettext_defs.h"
#include "nvim/macros_defs.h"
#include "nvim/math.h"
#include "nvim/mbyte.h"
#include "nvim/mbyte_defs.h"
#include "nvim/memory.h"
#include "nvim/memory_defs.h"
#include "nvim/message.h"
#include "nvim/option.h"
#include "nvim/plines.h"
#include "nvim/strings.h"
#include "nvim/types_defs.h"
#include "nvim/vim_defs.h"

#include "strings.c.generated.h"

static const char e_aptypes_is_null_nr_str[]
  = "E1507: Internal error: ap_types or ap_types[idx] is NULL: %d: %s";

enum {
  TYPE_UNKNOWN = -1,
  TYPE_INT,
  TYPE_LONGINT,
  TYPE_LONGLONGINT,
  TYPE_SIGNEDSIZET,
  TYPE_UNSIGNEDINT,
  TYPE_UNSIGNEDLONGINT,
  TYPE_UNSIGNEDLONGLONGINT,
  TYPE_SIZET,
  TYPE_POINTER,
  TYPE_PERCENT,
  TYPE_CHAR,
  TYPE_STRING,
  TYPE_FLOAT,
};

enum { MAX_ALLOWED_STRING_WIDTH = 1048576, };  // 1MiB

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

static void skip_to_arg(const char **ap_types, va_list ap_start, va_list *ap, int *arg_idx,
                        int *arg_cur, const char *fmt)
  FUNC_ATTR_NONNULL_ARG(3, 4, 5)
{
  int arg_min = 0;

  if (*arg_cur + 1 == *arg_idx) {
    (*arg_cur)++;
    (*arg_idx)++;
    return;
  }

  if (*arg_cur >= *arg_idx) {
    // Reset ap to ap_start and skip arg_idx - 1 types
    va_end(*ap);
    va_copy(*ap, ap_start);
  } else {
    // Skip over any we should skip
    arg_min = *arg_cur;
  }

  for (*arg_cur = arg_min; *arg_cur < *arg_idx - 1; (*arg_cur)++) {
    if (ap_types == NULL || ap_types[*arg_cur] == NULL) {
      siemsg(e_aptypes_is_null_nr_str, fmt, *arg_cur);
      return;
    }

    const char *p = ap_types[*arg_cur];

    int fmt_type = format_typeof(p);

    // get parameter value, do initial processing
    switch (fmt_type) {
    case TYPE_PERCENT:
    case TYPE_UNKNOWN:
      break;

    case TYPE_CHAR:
      va_arg(*ap, int);
      break;

    case TYPE_STRING:
      va_arg(*ap, const char *);
      break;

    case TYPE_POINTER:
      va_arg(*ap, void *);
      break;

    case TYPE_INT:
      va_arg(*ap, int);
      break;

    case TYPE_LONGINT:
      va_arg(*ap, long);
      break;

    case TYPE_LONGLONGINT:
      va_arg(*ap, long long);  // NOLINT(runtime/int)
      break;

    case TYPE_SIGNEDSIZET:  // implementation-defined, usually ptrdiff_t
      va_arg(*ap, ptrdiff_t);
      break;

    case TYPE_UNSIGNEDINT:
      va_arg(*ap, unsigned);
      break;

    case TYPE_UNSIGNEDLONGINT:
      va_arg(*ap, unsigned long);
      break;

    case TYPE_UNSIGNEDLONGLONGINT:
      va_arg(*ap, unsigned long long);  // NOLINT(runtime/int)
      break;

    case TYPE_SIZET:
      va_arg(*ap, size_t);
      break;

    case TYPE_FLOAT:
      va_arg(*ap, double);
      break;
    }
  }

  // Because we know that after we return from this call,
  // a va_arg() call is made, we can pre-emptively
  // increment the current argument index.
  (*arg_cur)++;
  (*arg_idx)++;
}

/// Write formatted value to the string
///
/// @param[out]  str  String to write to.
/// @param[in]  str_m  String length.
/// @param[in]  fmt  String format.
/// @param[in]  ap  Values that should be formatted. Ignored if tvs is not NULL.
/// @param[in]  tvs  Values that should be formatted, for printf() Vimscript
///                  function. Must be NULL in other cases.
///
/// @return Number of bytes excluding NUL byte that would be written to the
///         string if str_m was greater or equal to the return value.
int vim_vsnprintf_typval(char *str, size_t str_m, const char *fmt, va_list ap_start,
                         typval_T *const tvs)
{
  size_t str_l = 0;
  bool str_avail = str_l < str_m;
  const char *p = fmt;
  int arg_cur = 0;
  int num_posarg = 0;
  int arg_idx = 1;
  va_list ap;
  const char **ap_types = NULL;

  if (parse_fmt_types(&ap_types, &num_posarg, fmt, tvs) == FAIL) {
    return 0;
  }

  va_copy(ap, ap_start);

  if (!p) {
    p = "";
  }
  while (*p) {
    if (*p != '%') {
      // copy up to the next '%' or NUL without any changes
      size_t n = (size_t)(xstrchrnul(p + 1, '%') - p);
      if (str_avail) {
        size_t avail = str_m - str_l;
        memmove(str + str_l, p, MIN(n, avail));
        str_avail = n < avail;
      }
      p += n;
      assert(n <= SIZE_MAX - str_l);
      str_l += n;
    } else {
      size_t min_field_width = 0;
      size_t precision = 0;
      bool zero_padding = false;
      bool precision_specified = false;
      bool justify_left = false;
      bool alternate_form = false;
      bool force_sign = false;

      // if both ' ' and '+' flags appear, ' ' flag should be ignored
      int space_for_positive = 1;

      // allowed values: \0, h, l, 2 (for ll), z, L
      char length_modifier = NUL;

      // temporary buffer for simple numeric->string conversion
#define TMP_LEN 350    // 1e308 seems reasonable as the maximum printable
      char tmp[TMP_LEN];

      // string address in case of string argument
      const char *str_arg = NULL;

      // natural field width of arg without padding and sign
      size_t str_arg_l;

      // unsigned char argument value (only defined for c conversion);
      // standard explicitly states the char argument for the c
      // conversion is unsigned
      unsigned char uchar_arg;

      // number of zeros to be inserted for numeric conversions as
      // required by the precision or minimal field width
      size_t number_of_zeros_to_pad = 0;

      // index into tmp where zero padding is to be inserted
      size_t zero_padding_insertion_ind = 0;

      // current conversion specifier character
      char fmt_spec = NUL;

      // buffer for 's' and 'S' specs
      char *tofree = NULL;

      // variable for positional arg
      int pos_arg = -1;

      p++;  // skip '%'

      // First check to see if we find a positional
      // argument specifier
      const char *ptype = p;

      while (ascii_isdigit(*ptype)) {
        ptype++;
      }

      if (*ptype == '$') {
        // Positional argument
        const char *digstart = p;
        unsigned uj;

        if (get_unsigned_int(digstart, &p, &uj, tvs != NULL) == FAIL) {
          goto error;
        }

        pos_arg = (int)uj;

        p++;
      }

      // parse flags
      while (true) {
        switch (*p) {
        case '0':
          zero_padding = true; p++; continue;
        case '-':
          justify_left = true; p++; continue;
        // if both '0' and '-' flags appear, '0' should be ignored
        case '+':
          force_sign = true; space_for_positive = 0; p++; continue;
        case ' ':
          force_sign = true; p++; continue;
        // if both ' ' and '+' flags appear, ' ' should be ignored
        case '#':
          alternate_form = true; p++; continue;
        case '\'':
          p++; continue;
        default:
          break;
        }
        break;
      }

      // parse field width
      if (*p == '*') {
        const char *digstart = p + 1;

        p++;

        if (ascii_isdigit((int)(*p))) {
          // Positional argument field width
          unsigned uj;

          if (get_unsigned_int(digstart, &p, &uj, tvs != NULL) == FAIL) {
            goto error;
          }

          arg_idx = (int)uj;

          p++;
        }

        int j = (tvs
                 ? (int)tv_nr(tvs, &arg_idx)
                 : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                &arg_cur, fmt),
                    va_arg(ap, int)));

        if (j > MAX_ALLOWED_STRING_WIDTH) {
          if (tvs != NULL) {
            format_overflow_error(digstart);
            goto error;
          } else {
            j = MAX_ALLOWED_STRING_WIDTH;
          }
        }

        if (j >= 0) {
          min_field_width = (size_t)j;
        } else {
          min_field_width = (size_t)-j;
          justify_left = true;
        }
      } else if (ascii_isdigit((int)(*p))) {
        // size_t could be wider than unsigned int; make sure we treat
        // argument like common implementations do
        const char *digstart = p;
        unsigned uj;

        if (get_unsigned_int(digstart, &p, &uj, tvs != NULL) == FAIL) {
          goto error;
        }

        min_field_width = uj;
      }

      // parse precision
      if (*p == '.') {
        p++;
        precision_specified = true;

        if (ascii_isdigit((int)(*p))) {
          // size_t could be wider than unsigned int; make sure we
          // treat argument like common implementations do
          const char *digstart = p;
          unsigned uj;

          if (get_unsigned_int(digstart, &p, &uj, tvs != NULL) == FAIL) {
            goto error;
          }

          precision = uj;
        } else if (*p == '*') {
          const char *digstart = p;

          p++;

          if (ascii_isdigit((int)(*p))) {
            // positional argument
            unsigned uj;

            if (get_unsigned_int(digstart, &p, &uj, tvs != NULL) == FAIL) {
              goto error;
            }

            arg_idx = (int)uj;

            p++;
          }

          int j = (tvs
                   ? (int)tv_nr(tvs, &arg_idx)
                   : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                  &arg_cur, fmt),
                      va_arg(ap, int)));

          if (j > MAX_ALLOWED_STRING_WIDTH) {
            if (tvs != NULL) {
              format_overflow_error(digstart);
              goto error;
            } else {
              j = MAX_ALLOWED_STRING_WIDTH;
            }
          }

          if (j >= 0) {
            precision = (size_t)j;
          } else {
            precision_specified = false;
            precision = 0;
          }
        }
      }

      // parse 'h', 'l', 'll' and 'z' length modifiers
      if (*p == 'h' || *p == 'l' || *p == 'z') {
        length_modifier = *p;
        p++;
        if (length_modifier == 'l' && *p == 'l') {
          // double l = long long
          length_modifier = 'L';
          p++;
        }
      }

      fmt_spec = *p;

      // common synonyms
      switch (fmt_spec) {
      case 'i':
        fmt_spec = 'd'; break;
      case 'D':
        fmt_spec = 'd'; length_modifier = 'l'; break;
      case 'U':
        fmt_spec = 'u'; length_modifier = 'l'; break;
      case 'O':
        fmt_spec = 'o'; length_modifier = 'l'; break;
      default:
        break;
      }

      switch (fmt_spec) {
      case 'd':
      case 'u':
      case 'o':
      case 'x':
      case 'X':
        if (tvs && length_modifier == NUL) {
          length_modifier = 'L';
        }
      }

      if (pos_arg != -1) {
        arg_idx = pos_arg;
      }

      // get parameter value, do initial processing
      switch (fmt_spec) {
      // '%' and 'c' behave similar to 's' regarding flags and field widths
      case '%':
      case 'c':
      case 's':
      case 'S':
        str_arg_l = 1;
        switch (fmt_spec) {
        case '%':
          str_arg = p;
          break;

        case 'c': {
          const int j = (tvs
                         ? (int)tv_nr(tvs, &arg_idx)
                         : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                        &arg_cur, fmt),
                            va_arg(ap, int)));

          // standard demands unsigned char
          uchar_arg = (unsigned char)j;
          str_arg = (char *)&uchar_arg;
          break;
        }

        case 's':
        case 'S':
          str_arg = (tvs
                     ? tv_str(tvs, &arg_idx, &tofree)
                     : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                    &arg_cur, fmt),
                        va_arg(ap, const char *)));

          if (!str_arg) {
#ifdef ENABLE_ASAN_UBSAN
            // this is only for vsnprintf() emulation, v:_null_string handled in tv_str()
            if (!tvs) {
              __sanitizer_print_stack_trace();
              __sanitizer_report_error_summary("SUMMARY: NULL argument passed to %s");
            }
#endif
            str_arg = "[NULL]";
            str_arg_l = 6;
          } else if (!precision_specified) {
            // make sure not to address string beyond the specified
            // precision
            str_arg_l = strlen(str_arg);
          } else if (precision == 0) {
            // truncate string if necessary as requested by precision
            str_arg_l = 0;
          } else {
            // memchr on HP does not like n > 2^31
            // TODO(elmart): check if this still holds / is relevant
            str_arg_l = (size_t)((char *)xmemscan(str_arg,
                                                  NUL,
                                                  MIN(precision,
                                                      0x7fffffff))
                                 - str_arg);
          }
          if (fmt_spec == 'S') {
            const char *p1;
            size_t i;

            for (i = 0, p1 = str_arg; *p1; p1 += utfc_ptr2len(p1)) {
              size_t cell = (size_t)utf_ptr2cells(p1);
              if (precision_specified && i + cell > precision) {
                break;
              }
              i += cell;
            }

            str_arg_l = (size_t)(p1 - str_arg);
            if (min_field_width != 0) {
              min_field_width += str_arg_l - i;
            }
          }
          break;

        default:
          break;
        }
        break;

      case 'd':
      case 'u':
      case 'b':
      case 'B':
      case 'o':
      case 'x':
      case 'X':
      case 'p': {
        // u, b, B, o, x, X and p conversion specifiers imply
        // the value is unsigned; d implies a signed value

        // 0 if numeric argument is zero (or if pointer is NULL for 'p'),
        // +1 if greater than zero (or non NULL for 'p'),
        // -1 if negative (unsigned argument is never negative)
        int arg_sign = 0;

        intmax_t arg = 0;
        uintmax_t uarg = 0;

        // only defined for p conversion
        const void *ptr_arg = NULL;

        if (fmt_spec == 'p') {
          ptr_arg = (tvs
                     ? tv_ptr(tvs, &arg_idx)
                     : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                    &arg_cur, fmt),
                        va_arg(ap, void *)));

          if (ptr_arg) {
            arg_sign = 1;
          }
        } else if (fmt_spec == 'b' || fmt_spec == 'B') {
          uarg = (tvs
                  ? (unsigned long long)tv_nr(tvs, &arg_idx)  // NOLINT(runtime/int)
                  : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                 &arg_cur, fmt),
                     va_arg(ap, unsigned long long)));  // NOLINT(runtime/int)
          arg_sign = (uarg != 0);
        } else if (fmt_spec == 'd') {
          // signed
          switch (length_modifier) {
          case NUL:
            arg = (tvs
                   ? (int)tv_nr(tvs, &arg_idx)
                   : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                  &arg_cur, fmt),
                      va_arg(ap, int)));
            break;
          case 'h':
            // char and short arguments are passed as int16_t
            arg = (int16_t)
                  (tvs
                   ? (int)tv_nr(tvs, &arg_idx)
                   : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                  &arg_cur, fmt),
                      va_arg(ap, int)));
            break;
          case 'l':
            arg = (tvs
                   ? (long)tv_nr(tvs, &arg_idx)
                   : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                  &arg_cur, fmt),
                      va_arg(ap, long)));
            break;
          case 'L':
            arg = (tvs
                   ? (long long)tv_nr(tvs, &arg_idx)  // NOLINT(runtime/int)
                   : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                  &arg_cur, fmt),
                      va_arg(ap, long long)));  // NOLINT(runtime/int)
            break;
          case 'z':  // implementation-defined, usually ptrdiff_t
            arg = (tvs
                   ? (ptrdiff_t)tv_nr(tvs, &arg_idx)
                   : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                  &arg_cur, fmt),
                      va_arg(ap, ptrdiff_t)));
            break;
          }
          if (arg > 0) {
            arg_sign = 1;
          } else if (arg < 0) {
            arg_sign = -1;
          }
        } else {
          // unsigned
          switch (length_modifier) {
          case NUL:
            uarg = (tvs
                    ? (unsigned)tv_nr(tvs, &arg_idx)
                    : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                   &arg_cur, fmt),
                       va_arg(ap, unsigned)));
            break;
          case 'h':
            uarg = (uint16_t)
                   (tvs
                    ? (unsigned)tv_nr(tvs, &arg_idx)
                    : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                   &arg_cur, fmt),
                       va_arg(ap, unsigned)));
            break;
          case 'l':
            uarg = (tvs
                    ? (unsigned long)tv_nr(tvs, &arg_idx)
                    : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                   &arg_cur, fmt),
                       va_arg(ap, unsigned long)));
            break;
          case 'L':
            uarg = (tvs
                    ? (unsigned long long)tv_nr(tvs, &arg_idx)  // NOLINT(runtime/int)
                    : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                   &arg_cur, fmt),
                       va_arg(ap, unsigned long long)));  // NOLINT(runtime/int)
            break;
          case 'z':
            uarg = (tvs
                    ? (size_t)tv_nr(tvs, &arg_idx)
                    : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                   &arg_cur, fmt),
                       va_arg(ap, size_t)));
            break;
          }
          arg_sign = (uarg != 0);
        }

        str_arg = tmp;
        str_arg_l = 0;

        // For d, i, u, o, x, and X conversions, if precision is specified,
        // '0' flag should be ignored. This is so with Solaris 2.6, Digital
        // UNIX 4.0, HPUX 10, Linux, FreeBSD, NetBSD; but not with Perl.
        if (precision_specified) {
          zero_padding = false;
        }

        if (fmt_spec == 'd') {
          if (force_sign && arg_sign >= 0) {
            tmp[str_arg_l++] = space_for_positive ? ' ' : '+';
          }
          // leave negative numbers for snprintf to handle, to
          // avoid handling tricky cases like (short int)-32768
        } else if (alternate_form) {
          if (arg_sign != 0 && (fmt_spec == 'x' || fmt_spec == 'X'
                                || fmt_spec == 'b' || fmt_spec == 'B')) {
            tmp[str_arg_l++] = '0';
            tmp[str_arg_l++] = fmt_spec;
          }
          // alternate form should have no effect for p * conversion, but ...
        }

        zero_padding_insertion_ind = str_arg_l;
        if (!precision_specified) {
          precision = 1;  // default precision is 1
        }
        if (precision == 0 && arg_sign == 0) {
          // when zero value is formatted with an explicit precision 0,
          // resulting formatted string is empty (d, i, u, b, B, o, x, X, p)
        } else {
          switch (fmt_spec) {
          case 'p':    // pointer
            str_arg_l += (size_t)snprintf(tmp + str_arg_l,
                                          sizeof(tmp) - str_arg_l,
                                          "%p", ptr_arg);
            break;
          case 'd':    // signed
            str_arg_l += (size_t)snprintf(tmp + str_arg_l,
                                          sizeof(tmp) - str_arg_l,
                                          "%" PRIdMAX, arg);
            break;
          case 'b':
          case 'B': {  // binary
            size_t bits = 0;
            for (bits = sizeof(uintmax_t) * 8; bits > 0; bits--) {
              if ((uarg >> (bits - 1)) & 0x1) {
                break;
              }
            }

            while (bits > 0) {
              tmp[str_arg_l++] = ((uarg >> --bits) & 0x1) ? '1' : '0';
            }
            break;
          }
          default: {  // unsigned
            // construct a simple format string for snprintf
            char f[] = "%" PRIuMAX;
            f[sizeof("%" PRIuMAX) - 1 - 1] = fmt_spec;
            assert(PRIuMAX[sizeof(PRIuMAX) - 1 - 1] == 'u');
            str_arg_l += (size_t)snprintf(tmp + str_arg_l,
                                          sizeof(tmp) - str_arg_l,
                                          f, uarg);
            break;
          }
          }
          assert(str_arg_l < sizeof(tmp));

          // include the optional minus sign and possible "0x" in the region
          // before the zero padding insertion point
          if (zero_padding_insertion_ind < str_arg_l
              && tmp[zero_padding_insertion_ind] == '-') {
            zero_padding_insertion_ind++;
          }
          if (zero_padding_insertion_ind + 1 < str_arg_l
              && tmp[zero_padding_insertion_ind] == '0'
              && (tmp[zero_padding_insertion_ind + 1] == 'x'
                  || tmp[zero_padding_insertion_ind + 1] == 'X'
                  || tmp[zero_padding_insertion_ind + 1] == 'b'
                  || tmp[zero_padding_insertion_ind + 1] == 'B')) {
            zero_padding_insertion_ind += 2;
          }
        }

        {
          size_t num_of_digits = str_arg_l - zero_padding_insertion_ind;

          if (alternate_form && fmt_spec == 'o'
              // unless zero is already the first character
              && !(zero_padding_insertion_ind < str_arg_l
                   && tmp[zero_padding_insertion_ind] == '0')) {
            // assure leading zero for alternate-form octal numbers
            if (!precision_specified || precision < num_of_digits + 1) {
              // precision is increased to force the first character to be
              // zero, except if a zero value is formatted with an explicit
              // precision of zero
              precision = num_of_digits + 1;
            }
          }
          // zero padding to specified precision?
          if (num_of_digits < precision) {
            number_of_zeros_to_pad = precision - num_of_digits;
          }
        }
        // zero padding to specified minimal field width?
        if (!justify_left && zero_padding) {
          const int n = (int)(min_field_width - (str_arg_l
                                                 + number_of_zeros_to_pad));
          if (n > 0) {
            number_of_zeros_to_pad += (size_t)n;
          }
        }
        break;
      }

      case 'f':
      case 'F':
      case 'e':
      case 'E':
      case 'g':
      case 'G': {
        // floating point
        char format[40];
        bool remove_trailing_zeroes = false;

        double f = (tvs
                    ? tv_float(tvs, &arg_idx)
                    : (skip_to_arg(ap_types, ap_start, &ap, &arg_idx,
                                   &arg_cur, fmt),
                       va_arg(ap, double)));

        double abs_f = f < 0 ? -f : f;

        if (fmt_spec == 'g' || fmt_spec == 'G') {
          // can't use %g directly, cause it prints "1.0" as "1"
          if ((abs_f >= 0.001 && abs_f < 10000000.0) || abs_f == 0.0) {
            fmt_spec = ASCII_ISUPPER(fmt_spec) ? 'F' : 'f';
          } else {
            fmt_spec = fmt_spec == 'g' ? 'e' : 'E';
          }
          remove_trailing_zeroes = true;
        }

        if (xisinf(f)
            || (strchr("fF", fmt_spec) != NULL && abs_f > 1.0e307)) {
          xstrlcpy(tmp, infinity_str(f > 0.0, fmt_spec,
                                     force_sign, space_for_positive),
                   sizeof(tmp));
          str_arg_l = strlen(tmp);
          zero_padding = false;
        } else if (xisnan(f)) {
          // Not a number: nan or NAN
          memmove(tmp, ASCII_ISUPPER(fmt_spec) ? "NAN" : "nan", 4);
          str_arg_l = 3;
          zero_padding = false;
        } else {
          // Regular float number
          format[0] = '%';
          size_t l = 1;
          if (force_sign) {
            format[l++] = space_for_positive ? ' ' : '+';
          }
          if (precision_specified) {
            size_t max_prec = TMP_LEN - 10;

            // make sure we don't get more digits than we have room for
            if ((fmt_spec == 'f' || fmt_spec == 'F') && abs_f > 1.0) {
              max_prec -= (size_t)log10(abs_f);
            }
            if (precision > max_prec) {
              precision = max_prec;
            }
            l += (size_t)snprintf(format + l, sizeof(format) - l, ".%d",
                                  (int)precision);
          }

          // Cast to char to avoid a conversion warning on Ubuntu 12.04.
          assert(l + 1 < sizeof(format));
          format[l] = (char)(fmt_spec == 'F' ? 'f' : fmt_spec);
          format[l + 1] = NUL;

          str_arg_l = (size_t)snprintf(tmp, sizeof(tmp), format, f);
          assert(str_arg_l < sizeof(tmp));

          if (remove_trailing_zeroes) {
            char *tp;

            // using %g or %G: remove superfluous zeroes
            if (fmt_spec == 'f' || fmt_spec == 'F') {
              tp = tmp + str_arg_l - 1;
            } else {
              tp = vim_strchr(tmp, fmt_spec == 'e' ? 'e' : 'E');
              if (tp) {
                // remove superfluous '+' and leading zeroes from exponent
                if (tp[1] == '+') {
                  // change "1.0e+07" to "1.0e07"
                  STRMOVE(tp + 1, tp + 2);
                  str_arg_l--;
                }
                int i = (tp[1] == '-') ? 2 : 1;
                while (tp[i] == '0') {
                  // change "1.0e07" to "1.0e7"
                  STRMOVE(tp + i, tp + i + 1);
                  str_arg_l--;
                }
                tp--;
              }
            }

            if (tp != NULL && !precision_specified) {
              // remove trailing zeroes, but keep the one just after a dot
              while (tp > tmp + 2 && *tp == '0' && tp[-1] != '.') {
                STRMOVE(tp, tp + 1);
                tp--;
                str_arg_l--;
              }
            }
          } else {
            // Be consistent: some printf("%e") use 1.0e+12 and some
            // 1.0e+012; remove one zero in the last case.
            char *tp = vim_strchr(tmp, fmt_spec == 'e' ? 'e' : 'E');
            if (tp && (tp[1] == '+' || tp[1] == '-') && tp[2] == '0'
                && ascii_isdigit(tp[3]) && ascii_isdigit(tp[4])) {
              STRMOVE(tp + 2, tp + 3);
              str_arg_l--;
            }
          }
        }
        if (zero_padding && min_field_width > str_arg_l
            && (tmp[0] == '-' || force_sign)) {
          // Padding 0's should be inserted after the sign.
          number_of_zeros_to_pad = min_field_width - str_arg_l;
          zero_padding_insertion_ind = 1;
        }
        str_arg = tmp;
        break;
      }

      default:
        // unrecognized conversion specifier, keep format string as-is
        zero_padding = false;  // turn zero padding off for non-numeric conversion
        justify_left = true;
        min_field_width = 0;  // reset flags

        // discard the unrecognized conversion, just keep
        // the unrecognized conversion character
        str_arg = p;
        str_arg_l = 0;
        if (*p) {
          str_arg_l++;  // include invalid conversion specifier
        }
        // unchanged if not at end-of-string
        break;
      }

      if (*p) {
        p++;  // step over the just processed conversion specifier
      }

      // insert padding to the left as requested by min_field_width;
      // this does not include the zero padding in case of numerical conversions
      if (!justify_left) {
        assert(str_arg_l <= SIZE_MAX - number_of_zeros_to_pad);
        if (min_field_width > str_arg_l + number_of_zeros_to_pad) {
          // left padding with blank or zero
          size_t pn = min_field_width - (str_arg_l + number_of_zeros_to_pad);
          if (str_avail) {
            size_t avail = str_m - str_l;
            memset(str + str_l, zero_padding ? '0' : ' ', MIN(pn, avail));
            str_avail = pn < avail;
          }
          assert(pn <= SIZE_MAX - str_l);
          str_l += pn;
        }
      }

      // zero padding as requested by the precision or by the minimal
      // field width for numeric conversions required?
      if (number_of_zeros_to_pad == 0) {
        // will not copy first part of numeric right now,
        // force it to be copied later in its entirety
        zero_padding_insertion_ind = 0;
      } else {
        // insert first part of numerics (sign or '0x') before zero padding
        if (zero_padding_insertion_ind > 0) {
          size_t zn = zero_padding_insertion_ind;
          if (str_avail) {
            size_t avail = str_m - str_l;
            memmove(str + str_l, str_arg, MIN(zn, avail));
            str_avail = zn < avail;
          }
          assert(zn <= SIZE_MAX - str_l);
          str_l += zn;
        }

        // insert zero padding as requested by precision or min field width
        size_t zn = number_of_zeros_to_pad;
        if (str_avail) {
          size_t avail = str_m - str_l;
          memset(str + str_l, '0', MIN(zn, avail));
          str_avail = zn < avail;
        }
        assert(zn <= SIZE_MAX - str_l);
        str_l += zn;
      }

      // insert formatted string
      // (or as-is conversion specifier for unknown conversions)
      if (str_arg_l > zero_padding_insertion_ind) {
        size_t sn = str_arg_l - zero_padding_insertion_ind;
        if (str_avail) {
          size_t avail = str_m - str_l;
          memmove(str + str_l,
                  str_arg + zero_padding_insertion_ind,
                  MIN(sn, avail));
          str_avail = sn < avail;
        }
        assert(sn <= SIZE_MAX - str_l);
        str_l += sn;
      }

      // insert right padding
      if (justify_left) {
        assert(str_arg_l <= SIZE_MAX - number_of_zeros_to_pad);
        if (min_field_width > str_arg_l + number_of_zeros_to_pad) {
          // right blank padding to the field width
          size_t pn = min_field_width - (str_arg_l + number_of_zeros_to_pad);
          if (str_avail) {
            size_t avail = str_m - str_l;
            memset(str + str_l, ' ', MIN(pn, avail));
            str_avail = pn < avail;
          }
          assert(pn <= SIZE_MAX - str_l);
          str_l += pn;
        }
      }

      xfree(tofree);
    }
  }

  if (str_m > 0) {
    // make sure the string is nul-terminated even at the expense of
    // overwriting the last character (shouldn't happen, but just in case)
    str[str_l <= str_m - 1 ? str_l : str_m - 1] = NUL;
  }

  if (tvs != NULL
      && tvs[num_posarg != 0 ? num_posarg : arg_idx - 1].v_type != VAR_UNKNOWN) {
    emsg(_("E767: Too many arguments to printf()"));
  }

error:
  xfree(ap_types);
  va_end(ap);

  // return the number of characters formatted (excluding trailing nul
  // character); that is, the number of characters that would have been
  // written to the buffer if it were large enough.
  return (int)str_l;
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
