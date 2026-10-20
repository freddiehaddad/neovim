#pragma once

#include <stddef.h>  // IWYU pragma: keep

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/garray_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// Flags for expand_wildcards()
enum {
  EW_DIR        = 0x01,     ///< include directory names
  EW_FILE       = 0x02,     ///< include file names
  EW_NOTFOUND   = 0x04,     ///< include not found names
  EW_ADDSLASH   = 0x08,     ///< append slash to directory name
  EW_KEEPALL    = 0x10,     ///< keep all matches
  EW_SILENT     = 0x20,     ///< don't print "1 returned" from shell
  EW_EXEC       = 0x40,     ///< executable files
  EW_PATH       = 0x80,     ///< search in 'path' too
  EW_ICASE      = 0x100,    ///< ignore case
  EW_NOERROR    = 0x200,    ///< no error for bad regexp
  EW_NOTWILD    = 0x400,    ///< add match with literal name if exists
  EW_KEEPDOLLAR = 0x800,    ///< do not escape $, $var is expanded
  EW_ALLLINKS   = 0x1000,   ///< also links not pointing to existing file
  EW_SHELLCMD   = 0x2000,   ///< called from expand_shellcmd(), don't check
                            ///< if executable is in $PATH
  EW_DODOT      = 0x4000,   ///< also files starting with a dot
  EW_EMPTYOK    = 0x8000,   ///< no matches is not an error
  EW_NOTENV     = 0x10000,  ///< do not expand environment variables
  EW_CDPATH     = 0x20000,  ///< search in 'cdpath' too
  EW_NOBREAK    = 0x40000,  ///< do not invoke breakcheck
};
// Note: mostly EW_NOTFOUND and EW_SILENT are mutually exclusive: EW_NOTFOUND
// is used when executing commands and EW_SILENT for interactive expanding.

/// path_equal() flags
typedef enum {
  kPathCmpLiteral = 1 << 0,  ///< Compare paths literally.
  kPathCmpExpand  = 1 << 1,  ///< Expand env vars in the first path.
  kPathCmpFull    = 1 << 2,  ///< Compare full names when FileIds are unavailable.
} PathCmpFlags;

#ifdef BACKSLASH_IN_FILENAME
# define TO_SLASH(p) path_to_slash(p)
# define TO_SLASH_SAVE(p) path_to_slash_save(p)
# define TO_BACKSLASH(p) path_to_backslash(p)
#else
# define TO_SLASH(...)
# define TO_SLASH_SAVE(p) xstrdup(p)
# define TO_BACKSLASH(...)
#endif

#ifdef MSWIN
# define PATH_ESC_WILDCARDS "*?["
# define PATH_ALL_WILDCARDS "*?[`$"
#else
# define PATH_ESC_WILDCARDS "*?[{"
# define PATH_ALL_WILDCARDS "*?[{`'$"
#endif

#include "nvim/func_attr.h"

DLLEXPORT bool path_equal(const char *s1, const char *s2, PathCmpFlags flags) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *path_tail(const char *fname) FUNC_ATTR_NONNULL_RET;
DLLEXPORT char *path_tail_with_sep(char *fname) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT const char *invocation_path_tail(const char *invocation, size_t *len)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT const char *path_next_component(const char *fname) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *path_skip_sep(const char *path, bool colon)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET FUNC_ATTR_PURE;
DLLEXPORT int path_head_length(void);
DLLEXPORT bool is_path_head(const char *path) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *get_past_head(const char *path) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool vim_ispathsep(int c);
DLLEXPORT bool vim_ispathsep_nocolon(int c);
DLLEXPORT bool vim_ispathlistsep(int c);
DLLEXPORT void shorten_dir_len(char *str, int trim_len) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void shorten_dir(char *str) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool dir_of_file_exists(char *fname) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT String concat_fnames(String fname1, String fname2, bool sep) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *concat_fnames_realloc(char *fname1, const char *fname2, bool sep)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
DLLEXPORT bool add_pathsep(char *p) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *FullName_save(const char *fname, bool force) FUNC_ATTR_MALLOC;
DLLEXPORT char *save_abs_path(const char *name) FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool path_has_wildcard(const char *p, bool all) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT const char *gettail_dir(const char *const fname)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int gen_expand_wildcards(int num_pat, char **pat, int *num_file, char ***file, int flags);
DLLEXPORT void FreeWild(int count, char **files);
#ifdef BACKSLASH_IN_FILENAME
DLLEXPORT void slash_adjust(char *p) FUNC_ATTR_NONNULL_ALL;
#endif
DLLEXPORT char *path_to_backslash(char *p);
DLLEXPORT char *path_to_slash(char *p);
DLLEXPORT char *path_to_slash_save(const char *p);
DLLEXPORT void addfile(garray_T *gap, char *f, int flags) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT size_t simplify_filename(char *filename) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool path_has_drive_letter(const char *p, size_t path_len) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int path_is_url(const char *p) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int path_with_url(const char *fname) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool path_with_extension(const char *path, const char *extension) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool vim_isAbsName(const char *name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int vim_FullName(const char *fname, char *buf, size_t len, bool force)
  FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT char *fix_fname(const char *fname);
DLLEXPORT void path_fix_case(char *name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int after_pathsep(const char *b, const char *p) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool same_directory(char *f1, char *f2);
DLLEXPORT int path_fold_char(bool ic, const char *p, int *len) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int path_cmp(bool ic, const char *p, const char *q, size_t maxlen);
DLLEXPORT char *path_try_shorten_fname(char *full_path);
DLLEXPORT char *path_shorten_fname(char *full_path, char *dir_name);
DLLEXPORT int expand_wildcards_eval(char **pat, int *num_file, char ***file, int flags);
DLLEXPORT int expand_wildcards(int num_pat, char **pat, int *num_files, char ***files, int flags);
DLLEXPORT bool match_suffix(char *fname) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int path_full_dir_name(char *directory, char *buffer, size_t len) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int append_path(char *path, const char *to_append, size_t max_len) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool path_is_absolute(const char *fname) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void path_guess_exepath(const char *argv0, char *buf, size_t bufsize)
  FUNC_ATTR_NONNULL_ALL;
