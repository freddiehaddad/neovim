#pragma once

#include <stddef.h>  // IWYU pragma: keep

#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep
#include "nvim/vim_defs.h"  // IWYU pragma: keep

/// Flags for find_file_*() functions.
enum {
  FINDFILE_FILE = 0,  ///< only files
  FINDFILE_DIR  = 1,  ///< only directories
  FINDFILE_BOTH = 2,  ///< files and directories
};

/// Values for file_name_in_line()
enum {
  FNAME_MESS  = 1,   ///< give error message
  FNAME_EXP   = 2,   ///< expand to path
  FNAME_HYP   = 4,   ///< check for hyperlink url ("foo://")
  FNAME_INCL  = 8,   ///< apply 'includeexpr'
  FNAME_REL   = 16,  ///< ".." and "./" are relative to the (current)
                     ///< file instead of the current directory
  FNAME_UNESC = 32,  ///< remove backslashes used for escaping
};

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void *vim_findfile_init(char *path, char *filename, size_t filenamelen, char *stopdirs,
                                  int level, int free_visited, int find_what, void *search_ctx_arg,
                                  int tagfile, char *rel_fname);
DLLEXPORT char *vim_findfile_stopdir(char *buf);
DLLEXPORT void vim_findfile_cleanup(void *ctx);
DLLEXPORT char *vim_findfile(void *search_ctx_arg);
DLLEXPORT char *find_file_in_path(char *ptr, size_t len, int options, int first, char *rel_fname,
                                  char **file_to_find, char **search_ctx);
DLLEXPORT void free_findfile(void);
DLLEXPORT char *find_directory_in_path(char *ptr, size_t len, int options, char *rel_fname,
                                       char **file_to_find, char **search_ctx);
DLLEXPORT char *find_file_in_path_option(char *ptr, size_t len, int options, int first,
                                         char *path_option, int find_what, char *rel_fname,
                                         char *suffixes, char **file_to_find,
                                         char **search_ctx_arg);
DLLEXPORT char *grab_file_name(int count, linenr_T *file_lnum);
DLLEXPORT char *file_name_at_cursor(int options, int count, linenr_T *file_lnum);
DLLEXPORT char *file_name_in_line(char *line, int col, int options, int count, char *rel_fname,
                                  linenr_T *file_lnum);
DLLEXPORT char *find_file_name_in_path(char *ptr, size_t len, int options, long count,
                                       char *rel_fname);
DLLEXPORT void do_autocmd_dirchanged(char *new_dir, CdScope scope, CdCause cause, bool pre);
DLLEXPORT int vim_chdirfile(char *fname, CdCause cause);
DLLEXPORT int vim_chdir(char *new_dir);
