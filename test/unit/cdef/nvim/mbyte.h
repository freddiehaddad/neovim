#pragma once

#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <sys/types.h>  // IWYU pragma: keep
#include <utf8proc.h>
#include <uv.h>  // IWYU pragma: keep

#include "nvim/ascii_defs.h"
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/mbyte_defs.h"  // IWYU pragma: keep
#include "nvim/option_vars.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep

#define GRAPHEME_STATE_INIT 0

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

DLLEXPORT int enc_canon_props(const char *name) FUNC_ATTR_PURE;
DLLEXPORT int bomb_size(void) FUNC_ATTR_PURE;
DLLEXPORT void remove_bom(char *s) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int mb_get_class(const char *p) FUNC_ATTR_PURE;
DLLEXPORT int mb_get_class_tab(const char *p, const uint64_t *const chartab) FUNC_ATTR_PURE;
DLLEXPORT int utf_char2cells(int c) FUNC_ATTR_PURE;
DLLEXPORT int ptr2cells(const char *p_in);
DLLEXPORT int utf_ptr2cells(const char *p_in) FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int32_t utf_ptr2CharInfo_impl(uint8_t const *p, uintptr_t const len)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT size_t mb_string2cells(const char *str) FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT size_t mb_string2cells_len(const char *str, size_t size)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int utf_ptr2char(const char *const p_in)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int mb_ptr2char_adv(const char **const pp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int mb_cptr2char_adv(const char **pp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool utf_iscomposing_first(int c) FUNC_ATTR_PURE;
DLLEXPORT bool utf_composinglike(const char *p1, const char *p2, GraphemeState *state)
  FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT bool utf_iscomposing(int c1, int c2, GraphemeState *state);
DLLEXPORT schar_T utfc_ptr2schar(const char *p, int *firstc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT schar_T utfc_ptrlen2schar(const char *p, int len, int *firstc) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int utf_ptr2len(const char *const p_in)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int utf_byte2len(int b) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int utf_ptr2len_len(const char *p, int size)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int utfc_ptr2len(const char *const p)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int utfc_ptr2len_len(const char *p, int size) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int utf_char2len(int c) FUNC_ATTR_CONST FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int utf_char2bytes(int c, char *const buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool utf_iscomposing_legacy(int c) FUNC_ATTR_PURE;
DLLEXPORT bool utf_isalnum(int c) FUNC_ATTR_PURE;
DLLEXPORT bool utf_printable(int c) FUNC_ATTR_CONST;
DLLEXPORT int utf_class(int c) FUNC_ATTR_PURE;
DLLEXPORT int utf_class_tab(int c, const uint64_t *const chartab) FUNC_ATTR_PURE;
DLLEXPORT bool utf_ambiguous_width(const char *p) FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int utf_fold(int a) FUNC_ATTR_PURE;
DLLEXPORT int mb_toupper(int a);
DLLEXPORT bool mb_islower(int a);
DLLEXPORT int mb_tolower(int a);
DLLEXPORT bool mb_isupper(int a);
DLLEXPORT bool mb_isalpha(int a) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int utf_strnicmp(const char *s1, const char *s2, size_t n1, size_t n2)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
#ifdef MSWIN
DLLEXPORT int utf8_to_utf16(const char *utf8, int utf8len, wchar_t **utf16) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int utf16_to_utf8(const wchar_t *utf16, int utf16len, char **utf8) FUNC_ATTR_NONNULL_ALL;
#endif
DLLEXPORT void mb_utflen(const char *s, size_t len, size_t *codepoints, size_t *codeunits)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT ssize_t mb_utf_index_to_bytes(const char *s, size_t len, size_t index,
                                        bool use_utf16_units)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int mb_strnicmp(const char *s1, const char *s2, size_t nn)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int mb_stricmp(const char *s1, const char *s2)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void show_utf8(void);
DLLEXPORT int utf_head_off(const char *base_in, const char *p_in)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT ClusterInfo utf_ClusterInfo_impl(StrCharInfo base, int *max_len);
DLLEXPORT bool utf_eat_space(int cc) FUNC_ATTR_CONST FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool utf_allow_break_before(int cc) FUNC_ATTR_CONST FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool utf_allow_break_after(int cc) FUNC_ATTR_CONST FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool utf_allow_break(int cc, int ncc) FUNC_ATTR_CONST FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void mb_copy_char(const char **const fp, char **const tp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int mb_off_next(const char *base, const char *p) FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT CharBoundsOff utf_cp_bounds_len(char const *base, char const *p_in, int p_len)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT CharBoundsOff utf_cp_bounds(char const *base, char const *p_in)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void utf_find_illegal(void);
DLLEXPORT bool utf_valid_string(const char *s, const char *end)
  FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void mb_adjust_cursor(void);
DLLEXPORT void mb_check_adjust_col(void *win_);
DLLEXPORT char *mb_prevptr(char *line, char *p) FUNC_ATTR_PURE;
DLLEXPORT int mb_charlen(const char *str) FUNC_ATTR_PURE;
DLLEXPORT int mb_charlen_len(const char *str, int len) FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT const char *mb_unescape(const char **const pp)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *enc_skip(char *p) FUNC_ATTR_PURE FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *enc_canonize(char *enc) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT char *enc_locale(void);
DLLEXPORT void *my_iconv_open(char *to, char *from);
DLLEXPORT void f_iconv(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT int convert_setup(vimconv_T *vcp, char *from, char *to) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int convert_setup_ext(vimconv_T *vcp, char *from, bool from_unicode_is_utf8, char *to,
                                bool to_unicode_is_utf8) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT char *string_convert(const vimconv_T *const vcp, char *ptr, size_t *lenp);
DLLEXPORT char *string_convert_ext(const vimconv_T *const vcp, char *ptr, size_t *lenp,
                                   size_t *unconvlenp);
DLLEXPORT void f_setcellwidths(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcellwidths(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_charclass(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT char *get_encoding_name(expand_T *xp FUNC_ATTR_UNUSED, int idx) FUNC_ATTR_PURE;
DLLEXPORT int mb_strcmp_ic(bool ic, const char *s1, const char *s2)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;

#include "nvim/func_attr.h"

enum {
  kInvalidByteCells = 4,
};

// Return byte length of character that starts with byte "b".
// Returns 1 for a single-byte character.
// MB_BYTE2LEN_CHECK() can be used to count a special key as one byte.
// Don't call MB_BYTE2LEN(b) with b < 0 or b > 255!
#define MB_BYTE2LEN(b)         utf8len_tab[b]
#define MB_BYTE2LEN_CHECK(b)   (((b) < 0 || (b) > 255) ? 1 : utf8len_tab[b])

extern const uint8_t utf8len_tab_zero[256];

extern const uint8_t utf8len_tab[256];

// Use our own character-case definitions, because the current locale may
// differ from what the .spl file uses.
// These must not be called with negative number!
// Multi-byte implementation.  For Unicode we can call utf_*(), but don't do
// that for ASCII, because we don't want to use 'casemap' here.  Otherwise use
// the "w" library function for characters above 255.
#define SPELL_TOFOLD(c) ((c) >= 128 ? utf_fold(c) : (int)spelltab.st_fold[c])

#define SPELL_TOUPPER(c) ((c) >= 128 ? mb_toupper(c) : (int)spelltab.st_upper[c])

#define SPELL_ISUPPER(c) ((c) >= 128 ? mb_isupper(c) : spelltab.st_isu[c])

// MB_PTR_ADV(): advance a pointer to the next character, taking care of
// multi-byte characters if needed. Skip over composing chars.
#define MB_PTR_ADV(p)      (p += utfc_ptr2len((char *)p))

// MB_PTR_BACK(): backup a pointer to the previous character, taking care of
// multi-byte characters if needed. Only use with "p" > "s" !
#define MB_PTR_BACK(s, p) \
  (p -= utf_head_off((char *)(s), (char *)(p) - 1) + 1)

/// Check whether a given UTF-8 byte is a trailing byte (10xx.xxxx).

static inline bool utf_is_trail_byte(uint8_t const byte)
  FUNC_ATTR_CONST FUNC_ATTR_ALWAYS_INLINE
{
  // uint8_t is for clang to use smaller cmp
  return (uint8_t)(byte & 0xC0U) == 0x80U;
}

/// Convert a UTF-8 byte sequence to a Unicode code point.
/// Handles ascii, multibyte sequiences and illegal sequences.
///
/// @param[in]  p_in  String to convert.
///
/// @return information abouth the character. When the sequence is illegal,
/// "value" is negative, "len" is 1.
static inline CharInfo utf_ptr2CharInfo(char const *const p_in)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_ALWAYS_INLINE
{
  uint8_t const *const p = (uint8_t const *)p_in;
  uint8_t const first = *p;
  if (first < 0x80) {
    return (CharInfo){ .value = first, .len = 1 };
  } else {
    int len = utf8len_tab[first];
    int32_t const code_point = utf_ptr2CharInfo_impl(p, (uintptr_t)len);
    if (code_point < 0) {
      len = 1;
    }
    return (CharInfo){ .value = code_point, .len = len };
  }
}

static inline StrCharInfo utf_ptr2StrCharInfo(const char *ptr)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_PURE
{
  return (StrCharInfo){ .ptr = ptr, .chr = utf_ptr2CharInfo(ptr) };
}

static inline CharInfo utf_ptr2CharInfo_len(char const *const p_in, int size)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_ALWAYS_INLINE
{
  uint8_t const *const p = (uint8_t const *)p_in;
  assert(size >= 0);
  if (size == 0) {
    return (CharInfo){ .value = -1, .len = 1 };
  }

  uint8_t const first = *p;
  if (first < 0x80) {
    return (CharInfo){ .value = first, .len = 1 };
  } else {
    int len = utf8len_tab[first];
    if (len > size) {
      return (CharInfo){ .value = -1, .len = 1 };
    }
    int32_t const code_point = utf_ptr2CharInfo_impl(p, (uintptr_t)len);
    if (code_point < 0) {
      len = 1;
    }
    return (CharInfo){ .value = code_point, .len = len };
  }
}

static inline StrCharInfo utf_ptr2StrCharInfo_len(const char *ptr, int size)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_PURE
{
  return (StrCharInfo){ .ptr = ptr, .chr = utf_ptr2CharInfo_len(ptr, size) };
}

// only c.value is used here but it is a hint that it is the return value of ptr2CharInfo
// that is expected
static inline int basechar_cells_impl(CharInfo c)
{
  return c.value < 0 ? 4 : (c.value < 0x80 ? ascii2cells(c.value) : utf_char2cells(c.value));
}

/// Return information about the cluster width next character.
/// Composing and combining characters are considered a part of the current character.
///
/// @param[in] cur  Information about the current character in the string.
static inline ClusterInfo utf_ClusterInfo(StrCharInfo cur)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_PURE
{
  // most of the time time caller should already have checked for NUL, so
  // this will be inlined to nothing. But always be NUL safe.
  if (EXPECT(*cur.ptr == NUL, false)) {
    return (ClusterInfo) {
      .next = cur,
      .cells = dy_escape_width,
    };
  }

  uint8_t *next = (uint8_t *)(cur.ptr + cur.chr.len);
  // handle ASCII case inline
  if (EXPECT(*next < 0x80U, true)) {
    return (ClusterInfo) {
      .next = (StrCharInfo){
        .ptr = (char *)next,
        .chr = (CharInfo){ .value = *next, .len = 1 },
      },
      .cells = basechar_cells_impl(cur.chr)
    };
  }
  int scratch = INT_MAX;
  return utf_ClusterInfo_impl(cur, &scratch);
}

static inline ClusterInfo utf_ClusterInfo_len(StrCharInfo cur, int *size)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_ALWAYS_INLINE
{
  // TODO(bfredl): this is knowingly a bit inconsistent with utf_ClusterInfo
  // w.r.t control chars and truncated sequences
  // main consumer of mb_string2cells_len is currently str_to_reg, and once
  // it has been changed to behave more like buffer text, these edge cases
  // will disappear.

  *size -= cur.chr.len;
  uint8_t *next = (uint8_t *)(cur.ptr + cur.chr.len);
  if (EXPECT(*size == 0, false)) {
    return (ClusterInfo) {
      .next = (StrCharInfo){
        .ptr = (const char *)next,  // one past the end, but caller should check
        .chr = (CharInfo){ .value = -1, .len = 1 },
      },
      .cells = basechar_cells_impl(cur.chr),
    };
  }

  // handle ASCII case inline
  if (EXPECT(*next < 0x80U, true)) {
    return (ClusterInfo) {
      .next = (StrCharInfo){
        .ptr = (char *)next,
        .chr = (CharInfo){ .value = *next, .len = 1 },
      },
      .cells = basechar_cells_impl(cur.chr),
    };
  }
  return utf_ClusterInfo_impl(cur, size);
}

/// Return number of display cells occupied by ASCII byte "b".
///
/// chars outside of the range 0 <= b <= 127 is considered unprintable.
/// Use a proper wrapper for multibyte chars depending on the context, like
/// char2cells() or ptr2cells()
/// A TAB is counted as two or four cells: "^I" or "<09>".
///
/// @param b
///
/// @return Number of display cells.
static inline int ascii2cells(int b)
  FUNC_ATTR_PURE
{
  return (b >= ' ' && b <= '~') ? 1 : dy_escape_width;
}
