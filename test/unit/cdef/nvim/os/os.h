#pragma once

#include <stddef.h>
#include <stdint.h>
#include <uv.h>

#include "nvim/cmdexpand_defs.h"
#include "nvim/garray_defs.h"
#include "nvim/os/os_defs.h"
#include "nvim/os/stdpaths_defs.h"
#include "nvim/types_defs.h"

// True if when running in a test environment ($NVIM_TEST).
// TODO(justinmk): Can we use v:testing instead?
EXTERN bool nvim_testing INIT( = false);
extern char *default_vim_dir;
extern char *default_vimruntime_dir;
extern char *default_lib_dir;

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

DLLEXPORT void env_init(void);
DLLEXPORT size_t env_name_len(const char *p)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *os_getenv(const char *name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *os_getenv_buf(const char *const name, char *const buf,
                              const size_t bufsize) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *os_getenv_noalloc(const char *name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_env_exists(const char *name, bool nonempty) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_setenv(const char *name, const char *value, int overwrite) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int os_unsetenv(const char *name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void os_free_fullenv(char **env);
DLLEXPORT char *os_getenvname_at_index(size_t index);
DLLEXPORT int64_t os_get_pid(void);
DLLEXPORT void os_hint_priority(void);
DLLEXPORT void os_get_hostname(char *hostname, size_t size);
DLLEXPORT const char *os_homedir(void);
DLLEXPORT void init_homedir(void);
DLLEXPORT void free_homedir(void);
DLLEXPORT char *expand_env_save(char *src);
DLLEXPORT char *expand_env_save_opt(char *src, bool one, char *esc_chars);
DLLEXPORT size_t expand_env(char *src, char *dst, int dstlen);
DLLEXPORT size_t expand_env_esc(const char *restrict srcp, char *restrict dst, int dstlen,
                                char *esc_chars, bool one, char *prefix) FUNC_ATTR_NONNULL_ARG(1,
                                                                                               2);
DLLEXPORT const void *vim_env_iter(const char delim, const char *const val, const void *const iter,
                                   const char **const dir, size_t *const len)
  FUNC_ATTR_NONNULL_ARG(2, 4, 5) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT const void *vim_env_iter_rev(const char delim, const char *const val,
                                       const void *const iter, const char **const dir,
                                       size_t *const len)
  FUNC_ATTR_NONNULL_ARG(2, 4, 5) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void vim_get_prefix_from_exepath(char *exe_name);
DLLEXPORT char *vim_getenv(const char *name);
DLLEXPORT size_t home_replace(const buf_T *const buf, const char *src, char *const dst,
                              size_t dstlen, const bool one) FUNC_ATTR_NONNULL_ARG(3);
DLLEXPORT char *home_replace_save(buf_T *buf, const char *src) FUNC_ATTR_NONNULL_RET;
DLLEXPORT char *get_env_name(expand_T *xp, int idx);
DLLEXPORT bool os_setenv_append_path(const char *fname) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool os_shell_is_cmdexe(const char *sh) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void vim_unsetenv_ext(const char *var);
DLLEXPORT void vim_setenv_ext(const char *name, const char *val);
DLLEXPORT void restore_env_var(const char *name, char *old_value, bool must_free);

#include "nvim/func_attr.h"

uint64_t os_get_total_mem_kib(void);
const char *get_appname(bool namelike);
bool appname_is_valid(void);
char *stdpaths_get_xdg_var(const XDGVarType idx);
char *get_xdg_home(const XDGVarType idx);
char *stdpaths_user_cache_subpath(const char *fname);
char *stdpaths_user_conf_subpath(const char *fname);
char *stdpaths_user_data_subpath(const char *fname);
char *stdpaths_user_state_subpath(const char *fname, const size_t trailing_pathseps,
                                  const bool escape_commas);
int os_get_usernames(garray_T *users);
int os_get_username(char *s, size_t len);
int os_get_uname(uv_uid_t uid, char *s, size_t len);
char *os_get_userdir(const char *name);
void free_users(void);
char *get_users(expand_T *xp, int idx);
int match_user(char *name);

#define ENV_LOGFILE "NVIM_LOG_FILE"
#define ENV_LOGFILE_WANT "__NVIM_LOG_FILE_WANT"
#define ENV_NVIM "NVIM"
#define ENV_STARTREASON "__NVIM_STARTREASON"
#define ENV_TEST_LOG "__NVIM_TEST_LOG"
