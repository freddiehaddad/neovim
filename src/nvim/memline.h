#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "nvim/api/private/defs.h"
#include "nvim/ascii_defs.h"
#include "nvim/buffer_defs.h"
#include "nvim/eval/typval_defs.h"
#include "nvim/func_attr.h"
#include "nvim/memline_defs.h"  // IWYU pragma: keep
#include "nvim/os/os_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/types_defs.h"  // IWYU pragma: keep

int ml_open(buf_T *buf);
void ml_setname(buf_T *buf);
void ml_open_files(void);
void ml_open_file(buf_T *buf);
void check_need_swap(bool newfile);
void ml_close(buf_T *buf, int del_file);
void ml_close_all(bool del_file);
void ml_close_notmod(void);
void ml_timestamp(buf_T *buf);
void ml_recover(bool checkext);
void recover_names(char *fname, bool skip_curbuf, list_T *ret_list) FUNC_ATTR_NONNULL_ARG(3);
char *make_percent_swname(char *dir, char *dir_end, const char *name) FUNC_ATTR_NONNULL_ARG(1, 2);
void swapfile_dict(const char *fname, dict_T *d);
void ml_sync_all(int check_file, int check_char, bool do_fsync);
void ml_preserve(buf_T *buf, bool message, bool do_fsync);
char *ml_get(linenr_T lnum) FUNC_ATTR_NONNULL_RET;
char *ml_get_buf(buf_T *buf, linenr_T lnum) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
char *ml_get_buf_mut(buf_T *buf, linenr_T lnum) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_NONNULL_RET;
char *ml_get_pos(const pos_T *pos) FUNC_ATTR_NONNULL_ALL;
colnr_T ml_get_len(linenr_T lnum);
colnr_T ml_get_pos_len(pos_T *pos);
colnr_T ml_get_buf_len(buf_T *buf, linenr_T lnum);
String ml_region_text(buf_T *buf, pos_T start, pos_T end) FUNC_ATTR_NONNULL_ALL;
int gchar_pos(pos_T *pos) FUNC_ATTR_NONNULL_ARG(1);
int ml_line_alloced(void);
int ml_append(linenr_T lnum, char *line, colnr_T len, bool newfile);
int ml_append_flags(linenr_T lnum, char *line, colnr_T len, int flags);
int ml_append_buf(buf_T *buf, linenr_T lnum, char *line, colnr_T len, bool newfile)
  FUNC_ATTR_NONNULL_ARG(1);
void ml_add_deleted_len(char *ptr, ssize_t len);
void ml_add_deleted_len_buf(buf_T *buf, char *ptr, ssize_t len);
int ml_replace(linenr_T lnum, char *line, bool copy);
int ml_replace_len(linenr_T lnum, char *line, size_t len, bool copy);
int ml_replace_buf(buf_T *buf, linenr_T lnum, char *line, bool copy, bool noalloc)
  FUNC_ATTR_NONNULL_ARG(1);
int ml_replace_buf_len(buf_T *buf, linenr_T lnum, char *line_arg, size_t len_arg, bool copy,
                       bool noalloc) FUNC_ATTR_NONNULL_ARG(1);
int ml_delete_buf(buf_T *buf, linenr_T lnum, bool message) FUNC_ATTR_NONNULL_ALL;
int ml_delete(linenr_T lnum);
int ml_delete_flags(linenr_T lnum, int flags);
void ml_setmarked(linenr_T lnum);
linenr_T ml_firstmarked(void);
void ml_clearmarked(void);
size_t ml_flush_deleted_bytes(buf_T *buf, size_t *codepoints, size_t *codeunits);
#ifndef MSWIN
int resolve_symlink(const char *fname, char *buf);
#endif
char *makeswapname(char *fname, char *ffname, buf_T *buf, char *dir_name);
char *get_file_in_dir(char *fname, char *dname);
void ml_setflags(buf_T *buf);
int ml_find_line_or_offset(buf_T *buf, linenr_T lnum, int *offp, bool no_ff);
void goto_byte(int cnt);
int inc(pos_T *lp);
int incl(pos_T *lp);
int dec(pos_T *lp);
int decl(pos_T *lp);

/// LINEEMPTY() - return true if the line is empty
#define LINEEMPTY(p) (*ml_get(p) == NUL)

// Values for the flags argument of ml_delete_flags().
enum {
  ML_DEL_MESSAGE = 1,  // may give a "No lines in buffer" message
  // ML_DEL_UNDO = 2,  // called from undo
};

// Values for the flags argument of ml_append_int().
enum {
  ML_APPEND_NEW = 1,   // starting to edit a new file
  ML_APPEND_MARK = 2,  // mark the new line
  // ML_APPEND_UNDO = 4,  // called from undo
};
