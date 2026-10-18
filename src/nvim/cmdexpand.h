#pragma once

#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_getln_defs.h"  // IWYU pragma: keep
#include "nvim/garray_defs.h"  // IWYU pragma: keep
#include "nvim/regexp_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

// Values for nextwild() and ExpandOne().  See ExpandOne() for meaning.

enum {
  WILD_FREE        = 1,
  WILD_EXPAND_FREE = 2,
  WILD_EXPAND_KEEP = 3,
  WILD_NEXT        = 4,
  WILD_PREV        = 5,
  WILD_ALL         = 6,
  WILD_LONGEST     = 7,
  WILD_ALL_KEEP    = 8,
  WILD_CANCEL      = 9,
  WILD_APPLY       = 10,
  WILD_PAGEUP      = 11,
  WILD_PAGEDOWN    = 12,
  WILD_PUM_WANT    = 13,
};

enum {
  WILD_LIST_NOTFOUND        = 0x01,
  WILD_HOME_REPLACE         = 0x02,
  WILD_USE_NL               = 0x04,
  WILD_NO_BEEP              = 0x08,
  WILD_ADD_SLASH            = 0x10,
  WILD_KEEP_ALL             = 0x20,
  WILD_SILENT               = 0x40,
  WILD_ESCAPE               = 0x80,
  WILD_ICASE                = 0x100,
  WILD_ALLLINKS             = 0x200,
  WILD_USE_COMPLETESLASH    = 0x400,
  WILD_NOERROR              = 0x800,  ///< sets EW_NOERROR
  WILD_BUFLASTUSED          = 0x1000,
  BUF_DIFF_FILTER           = 0x2000,
  WILD_NOSELECT             = 0x4000,
  WILD_MAY_EXPAND_PATTERN   = 0x8000,
  WILD_FUNC_TRIGGER         = 0x10000,  ///< called from wildtrigger()
  WILD_NOINSERT             = 0x20000,
  WILD_USE_SHELLSLASH       = 0x40000,
};

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT bool cmdline_fuzzy_complete(const char *const fuzzystr)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE;
DLLEXPORT int nextwild(expand_T *xp, int type, int options, bool escape);
DLLEXPORT void cmdline_pum_display(bool changed_array);
DLLEXPORT bool cmdline_pum_active(void);
DLLEXPORT void cmdline_pum_remove(bool defer_redraw);
DLLEXPORT void cmdline_pum_cleanup(CmdlineInfo *cclp);
DLLEXPORT char *cmdline_compl_pattern(void);
DLLEXPORT bool cmdline_compl_is_fuzzy(void);
DLLEXPORT void free_xp_files_extra(expand_T *xp, int numfiles);
DLLEXPORT char *ExpandOne(expand_T *xp, char *str, char *orig, int options, int mode);
DLLEXPORT void ExpandInit(expand_T *xp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void ExpandCleanup(expand_T *xp);
DLLEXPORT void clear_cmdline_orig(void);
DLLEXPORT int showmatches(expand_T *xp, bool display_wildmenu, bool display_list,
                          int wim_flags_arg);
DLLEXPORT char *addstar(char *fname, size_t len, int context) FUNC_ATTR_NONNULL_RET;
DLLEXPORT void set_expand_context(expand_T *xp);
DLLEXPORT void set_cmd_context(expand_T *xp, char *str, int len, int col, int use_ccline);
DLLEXPORT int expand_cmdline(expand_T *xp, const char *str, int col, int *matchcount,
                             char ***matches);
DLLEXPORT void ExpandGeneric(const char *const pat, expand_T *xp, regmatch_T *regmatch,
                             char ***matches, int *numMatches, CompleteListItemGetter func,
                             bool escaped);
DLLEXPORT void expand_process_user_list(list_T *retlist, char ***matches, int *numMatches,
                                        expand_T *xp);
DLLEXPORT void globpath(char *path, char *file, garray_T *ga, int expand_options, bool dirs)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int wildmenu_translate_key(CmdlineInfo *cclp, int key, expand_T *xp, bool did_wild_list);
DLLEXPORT int wildmenu_process_key(CmdlineInfo *cclp, int key, expand_T *xp);
DLLEXPORT void wildmenu_cleanup(CmdlineInfo *cclp);
DLLEXPORT void f_getcompletion(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcompletiontype(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_cmdcomplete_info(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
