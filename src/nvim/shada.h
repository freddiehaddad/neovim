#pragma once

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"
#include "nvim/os/time_defs.h"  // IWYU pragma: keep

/// Flags for shada_read_file and children
typedef enum {
  kShaDaWantInfo = 1,       ///< Load non-mark information
  kShaDaWantMarks = 2,      ///< Load local file marks and change list
  kShaDaForceit = 4,        ///< Overwrite info already read
  kShaDaGetOldfiles = 8,    ///< Load v:oldfiles.
  kShaDaMissingError = 16,  ///< Error out when os_open returns -ENOENT.
  kShaDaNanos = 32,         ///< Timestamps are ns (Context), instead of seconds (shada legacy).
  kShaDaNoHistory = 64,     ///< Skip cmdline/search history merge (perf: O(history) per read).
  kShaDaNoOpt = 128,        ///< Ignore 'shada': only these flags decide what is read.
} ShaDaReadFileFlags;

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

DLLEXPORT int shada_write_file(const char *const file, bool nomerge);
DLLEXPORT int shada_read_marks(void);
DLLEXPORT int shada_read_everything(const char *const fname, const bool forceit,
                                    const bool missing_ok);
DLLEXPORT String shada_encode_regs(bool scale_ts, Timestamp since) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT String shada_encode_jumps(void) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT String shada_encode_buflist(void) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT String shada_encode_gvars(void) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void shada_read_string(String string, const int flags) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int get_shada_parameter(int type);
DLLEXPORT char *find_shada_parameter(int type);
DLLEXPORT void check_marks_read(void);

#include "nvim/func_attr.h"
