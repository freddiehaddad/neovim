#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/normal_defs.h"  // IWYU pragma: keep
#include "nvim/os/time_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/regexp_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"
#include "nvim/vim_defs.h"  // IWYU pragma: keep

/// Values for the find_pattern_in_path() function args 'type' and 'action':
enum {
  FIND_ANY    = 1,
  FIND_DEFINE = 2,
  CHECK_PATH  = 3,
};

enum {
  ACTION_SHOW     = 1,
  ACTION_GOTO     = 2,
  ACTION_SPLIT    = 3,
  ACTION_SHOW_ALL = 4,
  ACTION_EXPAND   = 5,
};

/// Values for "options" argument in do_search() and searchit()
enum {
  SEARCH_REV   = 0x01,    ///< go in reverse of previous dir.
  SEARCH_ECHO  = 0x02,    ///< echo the search command and handle options
  SEARCH_MSG   = 0x0c,    ///< give messages (yes, it's not 0x04)
  SEARCH_NFMSG = 0x08,    ///< give all messages except not found
  SEARCH_OPT   = 0x10,    ///< interpret optional flags
  SEARCH_HIS   = 0x20,    ///< put search pattern in history
  SEARCH_END   = 0x40,    ///< put cursor at end of match
  SEARCH_NOOF  = 0x80,    ///< don't add offset to position
  SEARCH_START = 0x100,   ///< start search without col offset
  SEARCH_MARK  = 0x200,   ///< set previous context mark
  SEARCH_KEEP  = 0x400,   ///< keep previous search pattern
  SEARCH_PEEK  = 0x800,   ///< peek for typed char, cancel search
  SEARCH_COL   = 0x1000,  ///< start at specified column instead of zero
};

/// Values for flags argument for findmatchlimit()
enum {
  FM_BACKWARD  = 0x01,  ///< search backwards
  FM_FORWARD   = 0x02,  ///< search forwards
  FM_BLOCKSTOP = 0x04,  ///< stop at start/end of block
  FM_SKIPCOMM  = 0x08,  ///< skip comments (cursor must start outside)
};

/// Values for sub_cmd and which_pat argument for search_regcomp()
/// Also used for which_pat argument for searchit()
enum {
  RE_SEARCH = 0,  ///< save/use pat in/from search_pattern
  RE_SUBST  = 1,  ///< save/use pat in/from subst_pattern
  RE_BOTH   = 2,  ///< save pat in both patterns
  RE_LAST   = 2,  ///< use last used pattern if "pat" is NULL
};

// Values for searchcount()
enum { SEARCH_STAT_DEF_TIMEOUT = 40, };
// 'W ':  2 +
// '[>9999/>9999]': 13 + 1 (NUL)
enum { SEARCH_STAT_BUF_LEN = 16, };

/// Structure containing offset definition for the last search pattern
///
/// @note Only offset for the last search pattern is used, not for the last
///       substitute pattern.
typedef struct {
  char dir;     ///< Search direction: forward ('/') or backward ('?')
  bool line;    ///< True if search has line offset.
  bool end;     ///< True if search sets cursor at the end.
  int64_t off;  ///< Actual offset value.
} SearchOffset;

/// Structure containing last search pattern and its attributes.
typedef struct {
  char *pat;            ///< The pattern (in allocated memory) or NULL.
  size_t patlen;        ///< The length of the pattern (0 if pat is NULL).
  bool magic;           ///< Magicness of the pattern.
  bool no_scs;          ///< No smartcase for this pattern.
  Timestamp timestamp;  ///< Time of the last change.
  SearchOffset off;     ///< Pattern offset.
  AdditionalData *additional_data;  ///< Additional data from ShaDa file.
} SearchPattern;

/// Optional extra arguments for searchit().
typedef struct {
  linenr_T sa_stop_lnum;  ///< stop after this line number when != 0
  proftime_T *sa_tm;        ///< timeout limit or NULL
  int sa_timed_out;  ///< set when timed out
  int sa_wrapped;    ///< search wrapped around
} searchit_arg_T;

typedef struct {
  int cur;      // current position of found words
  int cnt;      // total count of found words
  bool exact_match;    // true if matched exactly on specified position
  int incomplete;     // 0: search was fully completed
  // 1: recomputing was timed out
  // 2: max count exceeded
  int last_maxcount;  // the max count of the last search
} searchstat_T;

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

DLLEXPORT int search_regcomp(char *pat, size_t patlen, char **used_pat, int pat_save, int pat_use,
                             int options, bool magic, regmmatch_T *regmatch);
DLLEXPORT char *get_search_pat(void);
DLLEXPORT void save_re_pat(int idx, char *pat, size_t patlen, int magic);
DLLEXPORT void save_search_patterns(void);
DLLEXPORT void restore_search_patterns(void);
#ifdef EXITFREE
DLLEXPORT void free_search_patterns(void);
#endif
DLLEXPORT void save_last_search_pattern(void);
DLLEXPORT void restore_last_search_pattern(void);
DLLEXPORT char *last_search_pattern(void);
DLLEXPORT size_t last_search_pattern_len(void);
DLLEXPORT int ignorecase(char *pat);
DLLEXPORT int ignorecase_opt(char *pat, int ic_in, int scs);
DLLEXPORT bool pat_has_uppercase(char *pat) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT const char *last_csearch(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int last_csearch_forward(void);
DLLEXPORT int last_csearch_until(void);
DLLEXPORT void set_last_csearch(int c, char *s, int len);
DLLEXPORT void set_csearch_direction(Direction cdir);
DLLEXPORT void set_csearch_until(int t_cmd);
DLLEXPORT char *last_search_pat(void);
DLLEXPORT void reset_search_dir(void);
DLLEXPORT void set_last_search_pat(const char *s, int idx, int magic, bool setlast);
DLLEXPORT void last_pat_prog(regmmatch_T *regmatch);
DLLEXPORT int searchit(win_T *win, buf_T *buf, pos_T *pos, pos_T *end_pos, Direction dir, char *pat,
                       size_t patlen, int count, int options, int pat_use, bool magic,
                       searchit_arg_T *extra_arg);
DLLEXPORT void set_search_direction(int cdir);
DLLEXPORT int parse_search_pattern_offset(char **pat, size_t *patlen, int search_delim, int options,
                                          char **strcopy, char **searchstr, size_t *searchstrlen,
                                          char **dircp, SearchOffset *offset);
DLLEXPORT int do_search(oparg_T *oap, int dirc, int search_delim, char *pat, size_t patlen,
                        int count, int options, bool magic, searchit_arg_T *sia);
DLLEXPORT int search_for_exact_line(buf_T *buf, pos_T *pos, Direction dir, char *pat);
DLLEXPORT int searchc(cmdarg_T *cap, bool t_cmd) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT pos_T *findmatch(oparg_T *oap, int initc);
DLLEXPORT pos_T *findmatchlimit(oparg_T *oap, int initc, int flags, int64_t maxtravel);
DLLEXPORT void showmatch(int c);
DLLEXPORT int current_search(int count, bool forward);
DLLEXPORT bool linewhite(linenr_T lnum);
DLLEXPORT void f_searchcount(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void find_pattern_in_path(char *ptr, Direction dir, size_t len, bool whole,
                                    bool skip_comments, int type, int count, int action,
                                    linenr_T start_lnum, linenr_T end_lnum, bool forceit,
                                    bool silent);
DLLEXPORT void get_search_pattern(SearchPattern *const pat);
DLLEXPORT void get_substitute_pattern(SearchPattern *const pat);
DLLEXPORT Timestamp get_search_pattern_timestamp(bool substitute);
DLLEXPORT bool search_pattern_cleared(bool substitute);
DLLEXPORT void set_search_pattern(const SearchPattern pat);
DLLEXPORT void set_substitute_pattern(const SearchPattern pat);
DLLEXPORT void set_last_used_pattern(const bool is_substitute_pattern);
DLLEXPORT bool search_was_last_used(void);

#include "nvim/func_attr.h"
