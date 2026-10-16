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

// IWYU pragma: begin_exports
#include "os/env.h.generated.h"
#include "os/users.h.generated.h"
// IWYU pragma: end_exports

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

#define ENV_LOGFILE "NVIM_LOG_FILE"
#define ENV_LOGFILE_WANT "__NVIM_LOG_FILE_WANT"
#define ENV_NVIM "NVIM"
#define ENV_STARTREASON "__NVIM_STARTREASON"
#define ENV_TEST_LOG "__NVIM_TEST_LOG"
