#pragma once

#include <stdint.h>  // IWYU pragma: keep
#include <stdio.h>  // IWYU pragma: keep
#include <time.h>  // IWYU pragma: keep

#include "nvim/eval/typval_defs.h"
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/garray_defs.h"  // IWYU pragma: keep
#include "nvim/os/fs_defs.h"  // IWYU pragma: keep
#include "nvim/os/os_defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// Values for readfile() flags
enum {
  READ_NEW        = 0x01,   ///< read a file into a new buffer
  READ_FILTER     = 0x02,   ///< read filter output
  READ_STDIN      = 0x04,   ///< read from stdin
  READ_BUFFER     = 0x08,   ///< read from curbuf (converting stdin)
  READ_DUMMY      = 0x10,   ///< reading into a dummy buffer
  READ_KEEP_UNDO  = 0x20,   ///< keep undo info
  READ_FIFO       = 0x40,   ///< read from fifo or socket
  READ_NOWINENTER = 0x80,   ///< do not trigger BufWinEnter
  READ_NOFILE     = 0x100,  ///< do not read a file, do trigger BufReadCmd
};

typedef varnumber_T (*CheckItem)(void *expr, const char *name);

enum {
  FIO_LATIN1 = 0x01,       ///< convert Latin1
  FIO_UTF8 = 0x02,         ///< convert UTF-8
  FIO_UCS2 = 0x04,         ///< convert UCS-2
  FIO_UCS4 = 0x08,         ///< convert UCS-4
  FIO_UTF16 = 0x10,        ///< convert UTF-16
  FIO_ENDIAN_L = 0x80,     ///< little endian
  FIO_NOCONVERT = 0x2000,  ///< skip encoding conversion
  FIO_UCSBOM = 0x4000,     ///< check for BOM at start of file
  FIO_ALL = -1,            ///< allow all formats
};

enum {
  /// When converting, a read() or write() may leave some bytes to be converted
  /// for the next call.  The value is guessed...
  CONV_RESTLEN = 30,
};

enum { WRITEBUFSIZE = 8192, };  ///< size of normal write buffer

enum {
  /// We have to guess how much a sequence of bytes may expand when converting
  /// with iconv() to be able to allocate a buffer.
  ICONV_MULT = 8,
};

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

DLLEXPORT void filemess_bufwrite(buf_T *buf, char *name, const char *msg_id);
DLLEXPORT uint64_t slow_io_ns(void);
DLLEXPORT int readfile(char *fname, char *sfname, linenr_T from, linenr_T lines_to_skip,
                       linenr_T lines_to_read, exarg_T *eap, int flags, bool silent);
DLLEXPORT bool is_dev_fd_file(char *fname) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void prep_exarg(exarg_T *eap, const buf_T *buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void set_file_options(bool set_options, exarg_T *eap);
DLLEXPORT void set_forced_fenc(exarg_T *eap);
DLLEXPORT int set_rw_fname(char *fname, char *sfname);
DLLEXPORT void add_quoted_fname(char *const ret_buf, const size_t bufsize, const buf_T *const buf,
                                const char *fname) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void fname_msg_id(char *msg_id, size_t size, const char *prefix, const buf_T *buf,
                            const char *fname) FUNC_ATTR_NONNULL_ARG(1, 3);
DLLEXPORT bool msg_add_fileformat(int eol_type);
DLLEXPORT void msg_add_lines(int insert_space, linenr_T lnum, off_T nchars);
DLLEXPORT bool time_differs(const FileInfo *file_info, int64_t mtime, int64_t mtime_ns)
  FUNC_ATTR_CONST;
DLLEXPORT bool need_conversion(const char *fenc)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int get_fio_flags(const char *name);
DLLEXPORT bool shorten_buf_fname(buf_T *buf, char *dirname, int force);
DLLEXPORT void shorten_fnames(int force);
DLLEXPORT char *modname(const char *fname, const char *ext, bool prepend_dot)
  FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT bool vim_fgets(char *buf, int size, FILE *fp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int get2c(FILE *fd);
DLLEXPORT int get3c(FILE *fd);
DLLEXPORT int get4c(FILE *fd);
DLLEXPORT time_t get8ctime(FILE *fd);
DLLEXPORT char *read_string(FILE *fd, size_t cnt);
DLLEXPORT bool put_bytes(FILE *fd, uintmax_t number, size_t len);
DLLEXPORT int put_time(FILE *fd, time_t time_);
DLLEXPORT int vim_rename(const char *from, const char *to) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int vim_copyfile(const char *from, const char *to);
DLLEXPORT int check_timestamps(int focus);
DLLEXPORT int buf_check_timestamp(buf_T *buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void buf_reload(buf_T *buf, int orig_mode, bool reload_options);
DLLEXPORT void buf_store_file_info(buf_T *buf, FileInfo *file_info) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void write_lnum_adjust(linenr_T offset);
DLLEXPORT int readdir_core(garray_T *gap, const char *path, void *context, CheckItem checkitem)
  FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT int delete_recursive(const char *name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void vim_deltempdir(void);
DLLEXPORT char *vim_gettempdir(void);
DLLEXPORT char *vim_tempname(void);
DLLEXPORT bool match_file_pat(char *pattern, regprog_T **prog, char *fname, char *sfname,
                              char *tail, int allow_dirs);
DLLEXPORT bool match_file_list(char *list, char *sfname, char *ffname)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ARG(1, 3);
DLLEXPORT char *file_pat_to_reg_pat(const char *pat, const char *pat_end, char *allow_dirs,
                                    int no_bslash) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT ssize_t read_eintr(int fd, void *buf, size_t bufsize);
DLLEXPORT ssize_t write_eintr(int fd, void *buf, size_t bufsize);
#include "nvim/func_attr.h"
