#pragma once

#include <stdbool.h>
#include <stdio.h>

#include "auto/config.h"
#include "nvim/macros_defs.h"

// USDT probes. Example invocation:
//     NVIM_PROBE(nvim_foo_bar, 1, string.data);
#ifdef HAVE_SYS_SDT_H
# include <sys/sdt.h>  // IWYU pragma: keep

# define NVIM_PROBE(name, n, ...) STAP_PROBE##n(neovim, name, __VA_ARGS__)
#else
# define NVIM_PROBE(name, n, ...)
#endif

// uncrustify:off
#if NVIM_HAS_INCLUDE(<sanitizer/asan_interface.h>)
# include <sanitizer/asan_interface.h>  // IWYU pragma: keep
#endif
// uncrustify:on

#define LOGLVL_DBG 1
#define LOGLVL_INF 2
#define LOGLVL_WRN 3
#define LOGLVL_ERR 4

#define LOG(level, ...) logmsg((level), NULL, __func__, __LINE__, true, __VA_ARGS__)

#ifdef NVIM_LOG_DEBUG
# define DLOG(...) logmsg(LOGLVL_DBG, NULL, __func__, __LINE__, true, __VA_ARGS__)
# define DLOGN(...) logmsg(LOGLVL_DBG, NULL, __func__, __LINE__, false, __VA_ARGS__)
# define ILOG(...) logmsg(LOGLVL_INF, NULL, __func__, __LINE__, true, __VA_ARGS__)
# define ILOGN(...) logmsg(LOGLVL_INF, NULL, __func__, __LINE__, false, __VA_ARGS__)
#else
# define DLOG(...)
# define DLOGN(...)
# define ILOG(...)
# define ILOGN(...)
#endif

#define WLOG(...) logmsg(LOGLVL_WRN, NULL, __func__, __LINE__, true, __VA_ARGS__)
#define WLOGN(...) logmsg(LOGLVL_WRN, NULL, __func__, __LINE__, false, __VA_ARGS__)
#define ELOG(...) logmsg(LOGLVL_ERR, NULL, __func__, __LINE__, true, __VA_ARGS__)
#define ELOGN(...) logmsg(LOGLVL_ERR, NULL, __func__, __LINE__, false, __VA_ARGS__)

#ifdef HAVE_EXECINFO_BACKTRACE
# define LOG_CALLSTACK() log_callstack(__func__, __LINE__)
# define LOG_CALLSTACK_TO_FILE(fp) log_callstack_to_file(fp, __func__, __LINE__)
#endif

#ifndef NVIM_LOG_DEBUG
EXTERN int g_min_log_level INIT( = LOGLVL_WRN);
#else
EXTERN int g_min_log_level INIT( = 0);
#endif

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

DLLEXPORT void log_init(void);
DLLEXPORT void log_lock(void);
DLLEXPORT void log_unlock(void);
DLLEXPORT bool log_should_log(int log_level);
DLLEXPORT bool logmsg_str(int log_level, const char *context, const char *func_name, int line_num,
                          bool eol, const char *msg);
DLLEXPORT void log_uv_handles(void *loop);
DLLEXPORT FILE *open_log_file(void);
#ifndef MSWIN
DLLEXPORT void log_callstack_to_file_trace(FILE *log_file, const char *func_name, int line_num,
                                           void *const *trace, int trace_size);
#endif

#include "log.h.generated.h"
