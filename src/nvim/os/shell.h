#pragma once

#include <stddef.h>  // IWYU pragma: keep

// Flags for os_call_shell() second argument
typedef enum {
  kShellOptFilter = 1,     ///< filtering text
  kShellOptExpand = 2,     ///< expanding wildcards
  kShellOptDoOut = 4,      ///< redirecting output
  kShellOptSilent = 8,     ///< don't print error returned by command
  kShellOptRead = 16,      ///< read lines and insert into buffer
  kShellOptWrite = 32,     ///< write lines from buffer
  kShellOptHideMess = 64,  ///< previously a global variable from os_unix.c
} ShellOpts;

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

DLLEXPORT int os_expand_wildcards(int num_pat, char **pat, int *num_file, char ***file,
                                  int flags) FUNC_ATTR_NONNULL_ARG(3) FUNC_ATTR_NONNULL_ARG(4);
DLLEXPORT char **shell_build_argv(const char *cmd, const char *extra_args) FUNC_ATTR_NONNULL_RET;
DLLEXPORT void shell_free_argv(char **argv);
DLLEXPORT char *shell_argv_to_str(char **const argv) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_call_shell(char *cmd, int opts, char *extra_args);
DLLEXPORT int call_shell(char *cmd, int opts, char *extra_shell_arg);
DLLEXPORT char *get_cmd_output(char *cmd, char *infile, int flags, size_t *ret_len);
DLLEXPORT int os_system(char **argv, const char *input, size_t len, char **output,
                        size_t *nread) FUNC_ATTR_NONNULL_ARG(1);

#include "nvim/func_attr.h"
