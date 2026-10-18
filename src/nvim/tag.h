#pragma once

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

enum { LSIZE = 512, };  ///< max. size of a line in the tags file

/// Values for do_tag().
enum {
  DT_TAG    = 1,   ///< jump to newer position or same tag again
  DT_POP    = 2,   ///< jump to older position
  DT_NEXT   = 3,   ///< jump to next match of same tag
  DT_PREV   = 4,   ///< jump to previous match of same tag
  DT_FIRST  = 5,   ///< jump to first match of same tag
  DT_LAST   = 6,   ///< jump to first match of same tag
  DT_SELECT = 7,   ///< jump to selection from list
  DT_HELP   = 8,   ///< like DT_TAG, but no wildcards
  DT_JUMP   = 9,   ///< jump to new tag or selection from list
  DT_LTAG   = 11,  ///< tag using location list
  DT_FREE   = 99,  ///< free cached matches
};

/// flags for find_tags().
enum {
  TAG_HELP       = 1,    ///< only search for help tags
  TAG_NAMES      = 2,    ///< only return name of tag
  TAG_REGEXP     = 4,    ///< use tag pattern as regexp
  TAG_NOIC       = 8,    ///< don't always ignore case
  TAG_VERBOSE    = 32,   ///< message verbosity
  TAG_INS_COMP   = 64,   ///< Currently doing insert completion
  TAG_KEEP_LANG  = 128,  ///< keep current language
  TAG_NO_TAGFUNC = 256,  ///< do not use 'tagfunc'
  TAG_MANY       = 300,  ///< When finding many tags (for completion), find up to this many tags
};

/// Structure used for get_tagfname().
typedef struct {
  char *tn_tags;           ///< value of 'tags' when starting
  char *tn_np;             ///< current position in tn_tags
  int tn_did_filefind_init;
  int tn_hf_idx;
  void *tn_search_ctx;
} tagname_T;

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void free_tagfunc_option(void);
DLLEXPORT bool set_ref_in_tagfunc(int copyID);
DLLEXPORT void do_tag(exarg_T *eap, char *tag, int type, int count, int forceit, bool verbose);
DLLEXPORT void tag_freematch(void);
DLLEXPORT void do_tags(exarg_T *eap);
DLLEXPORT int find_tags(char *pat, int *num_matches, char ***matchesp, int flags, int mincount,
                        char *buf_ffname);
DLLEXPORT void free_tag_stuff(void);
DLLEXPORT int get_tagfname(tagname_T *tnp, int first, char *buf);
DLLEXPORT void tagname_free(tagname_T *tnp);
DLLEXPORT void tagstack_clear_entry(taggy_T *item);
DLLEXPORT int expand_tags(bool tagnames, char *pat, int *num_file, char ***file);
DLLEXPORT int get_tags(list_T *list, char *pat, char *buf_fname);
DLLEXPORT void get_tagstack(win_T *wp, dict_T *retdict);
DLLEXPORT int set_tagstack(win_T *wp, const dict_T *d, int action) FUNC_ATTR_NONNULL_ARG(1);
