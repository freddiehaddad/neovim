#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "nvim/option_vars.h"
#include "nvim/strings.h"  // IWYU pragma: keep

/// Flags for vim_str2nr()
typedef enum {
  STR2NR_DEC = 0,
  STR2NR_BIN = (1 << 0),  ///< Allow binary numbers.
  STR2NR_OCT = (1 << 1),  ///< Allow octal numbers.
  STR2NR_HEX = (1 << 2),  ///< Allow hexadecimal numbers.
  STR2NR_OOCT = (1 << 3),  ///< Octal with prefix "0o": 0o777
  /// Force one of the above variants.
  ///
  /// STR2NR_FORCE|STR2NR_DEC is actually not different from supplying zero
  /// as flags, but still present for completeness.
  ///
  /// STR2NR_FORCE|STR2NR_OCT|STR2NR_OOCT is the same as STR2NR_FORCE|STR2NR_OCT
  /// or STR2NR_FORCE|STR2NR_OOCT.
  STR2NR_FORCE = (1 << 7),
  /// Recognize all formats vim_str2nr() can recognize.
  STR2NR_ALL = STR2NR_BIN | STR2NR_OCT | STR2NR_HEX | STR2NR_OOCT,
  /// Disallow octals numbers without the 0o prefix.
  STR2NR_NO_OCT = STR2NR_BIN | STR2NR_HEX | STR2NR_OOCT,
  STR2NR_QUOTE = (1 << 4),  ///< Ignore embedded single quotes.
} ChStr2NrFlags;

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT int buf_init_isk_chartab(buf_T *buf);
DLLEXPORT int init_isf_chartab(void);
DLLEXPORT int init_isi_chartab(void);
DLLEXPORT int check_isopt(char *var);
DLLEXPORT int parse_isopt(const char *var, uint64_t *chartab);
DLLEXPORT void trans_characters(char *buf, int bufsize);
DLLEXPORT size_t transstr_len(const char *const s, bool untab)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
DLLEXPORT size_t transstr_buf(const char *const s, const ssize_t slen, char *const buf,
                              const size_t buflen, bool untab) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *transstr(const char *const s, bool untab) FUNC_ATTR_NONNULL_RET;
DLLEXPORT size_t kv_transstr(StringBuilder *str, const char *const s, bool untab)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT char *str_foldcase(char *str, int orglen, char *buf, int buflen) FUNC_ATTR_NONNULL_RET;
DLLEXPORT char *transchar(int c);
DLLEXPORT char *transchar_buf(const buf_T *buf, int c);
DLLEXPORT char *transchar_byte(const int c) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT char *transchar_byte_buf(const buf_T *buf, const int c) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void transchar_nonprint(const buf_T *buf, char *charbuf, int c);
DLLEXPORT size_t transchar_hex(char *const buf, const int c) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void rl_mirror_ascii(char *str, char *end);
DLLEXPORT int char2cells(int c);
DLLEXPORT int vim_strsize(const char *s);
DLLEXPORT int vim_strnsize(const char *s, int len);
DLLEXPORT bool vim_isIDc(int c) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool vim_iswordc(const int c) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool vim_iswordc_tab(const int c, const uint64_t *const chartab)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool vim_iswordc_buf(const int c, buf_T *const buf)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT bool vim_iswordp(const char *const p)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool vim_iswordp_buf(const char *const p, buf_T *const buf)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool vim_isfilec(int c) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool vim_is_fname_char(int c) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool vim_isfilec_or_wc(int c) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool vim_isprintc(int c) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT char *skipwhite(const char *p)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT char *skipwhite_len(const char *p, size_t len)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT intptr_t getwhitecols_curline(void);
DLLEXPORT intptr_t getwhitecols(const char *p) FUNC_ATTR_PURE;
DLLEXPORT char *skipdigits(const char *q)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT const char *skipbin(const char *q)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT char *skiphex(char *q) FUNC_ATTR_PURE;
DLLEXPORT char *skiptodigit(char *q) FUNC_ATTR_PURE;
DLLEXPORT const char *skiptobin(const char *q)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT char *skiptohex(char *q) FUNC_ATTR_PURE;
DLLEXPORT char *skiptowhite(const char *p) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE;
DLLEXPORT char *skiptowhite_esc(const char *p) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE;
DLLEXPORT char *skip_to_newline(const char *const p)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT bool try_getdigits(char **pp, intmax_t *nr);
DLLEXPORT intmax_t getdigits(char **pp, bool strict, intmax_t def);
DLLEXPORT int getdigits_int(char **pp, bool strict, int def);
DLLEXPORT long getdigits_long(char **pp, bool strict, long def);
DLLEXPORT int32_t getdigits_int32(char **pp, bool strict, int32_t def);
DLLEXPORT bool vim_isblankline(char *lbuf) FUNC_ATTR_PURE;
DLLEXPORT void vim_str2nr(const char *const start, int *const prep, int *const len, const int what,
                          varnumber_T *const nptr, uvarnumber_T *const unptr, const int maxlen,
                          const bool strict, bool *const overflow) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int hex2nr(int c) FUNC_ATTR_CONST;
DLLEXPORT int hexhex2nr(const char *p) FUNC_ATTR_PURE;
DLLEXPORT bool rem_backslash(const char *str)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void backslash_halve(char *p);
DLLEXPORT char *backslash_halve_save(const char *p) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;

#include "nvim/func_attr.h"

/// Check if `c` is one of the characters in 'breakat'.
/// Used very often if 'linebreak' is set
static inline bool vim_isbreak(int c)
  FUNC_ATTR_CONST FUNC_ATTR_ALWAYS_INLINE
{
  return breakat_flags[(uint8_t)c];
}
