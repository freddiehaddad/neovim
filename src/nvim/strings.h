#pragma once

#include <stdarg.h>  // IWYU pragma: keep
#include <string.h>

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/os/os_defs.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep

// Return the length of a string literal
#define STRLEN_LITERAL(s) (sizeof(s) - 1)

/// Store a key/value pair
typedef struct {
  int key;        ///< the key
  char *value;    ///< the value string
  size_t length;  ///< length of the value string
} keyvalue_T;

#define KEYVALUE_ENTRY(k, v) { (k), (v), STRLEN_LITERAL(v) }

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

DLLEXPORT char *xstrnsave(const char *string, size_t len)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *vim_strsave_escaped(const char *string, const char *esc_chars)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *vim_strsave_escaped_ext(const char *string, const char *esc_chars, char cc,
                                        bool bsl)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *vim_strnsave_unquoted(const char *const string, const size_t length)
  FUNC_ATTR_MALLOC FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT char *vim_strsave_shellescape(const char *string, bool do_special, bool do_newline)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *vim_strsave_up(const char *string)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *vim_strnsave_up(const char *string, size_t len)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void vim_strup(char *p) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void vim_strcpy_up(char *restrict dst, const char *restrict src) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void vim_strncpy_up(char *restrict dst, const char *restrict src, size_t n)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void vim_memcpy_up(char *restrict dst, const char *restrict src, size_t n)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT size_t mb_strup_buf(const char *src, char *dst) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *strcase_save(const char *const orig, bool upper)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void del_trailing_spaces(char *ptr) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int vim_stricmp(const char *s1, const char *s2) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE;
DLLEXPORT int vim_strnicmp(const char *s1, const char *s2, size_t len)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE;
DLLEXPORT bool striequal(const char *a, const char *b) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int vim_strnicmp_asc(const char *s1, const char *s2, size_t len)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT char *vim_strchr(const char *const string, const int c)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void sort_strings(char **files, int count);
DLLEXPORT bool has_non_ascii(const char *s) FUNC_ATTR_PURE;
DLLEXPORT bool has_non_ascii_len(const char *const s, const size_t len) FUNC_ATTR_PURE;
DLLEXPORT char *concat_str(const char *restrict str1, const char *restrict str2)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT varnumber_T tv_nr(typval_T *tvs, int *idxp);
DLLEXPORT const char *tv_str(typval_T *tvs, int *idxp, char **const tofree);
DLLEXPORT const void *tv_ptr(const typval_T *const tvs, int *const idxp);
DLLEXPORT float_T tv_float(typval_T *const tvs, int *const idxp);
DLLEXPORT const char *infinity_str(bool positive, char fmt_spec, int force_sign,
                                   int space_for_positive);
DLLEXPORT int format_typeof(const char *type);
DLLEXPORT void format_overflow_error(const char *pstart);
DLLEXPORT int get_unsigned_int(const char *pstart, const char **p, unsigned *uj, bool overflow_err);
DLLEXPORT int parse_fmt_types(const char ***ap_types, int *num_posarg, const char *fmt,
                              typval_T *tvs);
DLLEXPORT char *reverse_text(char *s) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT char *strrep(const char *src, const char *what, const char *rep);
DLLEXPORT void f_byteidx(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_byteidxcomp(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_charidx(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_str2list(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_str2nr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strgetchar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_stridx(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_string(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strlen(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strcharlen(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strchars(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strutf16len(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strdisplaywidth(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strwidth(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strcharpart(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strpart(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strridx(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_strtrans(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_utf16idx(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_tolower(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_toupper(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_tr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_trim(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT int cmp_keyvalue_value(const void *a, const void *b);
DLLEXPORT int cmp_keyvalue_value_n(const void *a, const void *b);
typedef struct {
  int tag;
  int64_t i;
  uint64_t u;
  double f;
  const void *p;
} VimFmtArg;

DLLEXPORT int cmp_keyvalue_value_i(const void *a, const void *b);
DLLEXPORT int cmp_keyvalue_value_ni(const void *a, const void *b);
DLLEXPORT int vim_snprintf_args(char *str, size_t str_m, const char *fmt, const VimFmtArg *args,
                                size_t argc);

#include "nvim/func_attr.h"

/// Append string to string and return pointer to the next byte
///
/// Unlike strcat, this one does *not* add NUL byte and returns pointer to the
/// past of the added string.
///
/// @param[out]  dst  String to append to.
/// @param[in]  src  String to append.
///
/// @return pointer to the byte just past the appended byte.
static inline char *strappend(char *const dst, const char *const src)
  FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_NONNULL_ALL
    FUNC_ATTR_NONNULL_RET FUNC_ATTR_WARN_UNUSED_RESULT
{
  const size_t src_len = strlen(src);
  return (char *)memmove(dst, src, src_len) + src_len;
}

#ifndef MSWIN
# define STRICMP(d, s)      strcasecmp((char *)(d), (char *)(s))
# define STRNICMP(d, s, n)  strncasecmp((char *)(d), (char *)(s), (n))
#else
# define STRICMP(d, s)      vim_stricmp((char *)(d), (char *)(s))
# define STRNICMP(d, s, n)  vim_strnicmp((char *)(d), (char *)(s), (n))
#endif
