#pragma once

#include <stddef.h>  // IWYU pragma: keep

#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// flags for open_line()
enum {
  OPENLINE_DELSPACES    = 0x01,  ///< delete spaces after cursor
  OPENLINE_DO_COM       = 0x02,  ///< format comments
  OPENLINE_KEEPTRAIL    = 0x04,  ///< keep trailing spaces
  OPENLINE_MARKFIX      = 0x08,  ///< fix mark positions
  OPENLINE_COM_LIST     = 0x10,  ///< format comments with list/2nd line indent
  OPENLINE_FORMAT       = 0x20,  ///< formatting long comment
  OPENLINE_FORCE_INDENT = 0x40,  ///< use second_line_indent without indent logic
};

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void change_warning(buf_T *buf, int col);
DLLEXPORT void changed(buf_T *buf);
DLLEXPORT void changed_internal(buf_T *buf);
DLLEXPORT void changed_lines_invalidate_buf(buf_T *buf, linenr_T lnum, colnr_T col, linenr_T lnume,
                                            linenr_T xtra);
DLLEXPORT void changed_bytes(linenr_T lnum, colnr_T col);
DLLEXPORT void inserted_bytes(linenr_T lnum, colnr_T start_col, int old_col, int new_col);
DLLEXPORT void appended_lines_buf(buf_T *buf, linenr_T lnum, linenr_T count);
DLLEXPORT void appended_lines(linenr_T lnum, linenr_T count);
DLLEXPORT void appended_lines_mark(linenr_T lnum, int count);
DLLEXPORT void deleted_lines_buf(buf_T *buf, linenr_T lnum, linenr_T count);
DLLEXPORT void deleted_lines(linenr_T lnum, linenr_T count);
DLLEXPORT void deleted_lines_mark(linenr_T lnum, int count);
DLLEXPORT void changed_lines_redraw_buf(buf_T *buf, linenr_T lnum, linenr_T lnume, linenr_T xtra);
DLLEXPORT void changed_lines(buf_T *buf, linenr_T lnum, colnr_T col, linenr_T lnume, linenr_T xtra,
                             bool do_buf_event);
DLLEXPORT void unchanged(buf_T *buf, bool ff, bool always_inc_changedtick);
DLLEXPORT void save_file_ff(buf_T *buf);
DLLEXPORT bool file_ff_differs(buf_T *buf, bool ignore_empty)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void ins_bytes(char *p);
DLLEXPORT void ins_bytes_len(char *p, size_t len);
DLLEXPORT void ins_char(int c);
DLLEXPORT void ins_char_bytes(char *buf, size_t charlen);
DLLEXPORT void ins_str(char *s, size_t slen);
DLLEXPORT int del_char(bool fixpos);
DLLEXPORT int del_chars(int count, int fixpos);
DLLEXPORT int del_bytes(colnr_T count, bool fixpos_arg, bool use_delcombine);
DLLEXPORT bool open_line(int dir, int flags, int second_line_indent, bool *did_do_comment);
DLLEXPORT void truncate_line(int fixpos);
DLLEXPORT void del_lines(linenr_T nlines, bool undo);
DLLEXPORT int get_leader_len(char *line, char **flags, bool backward, bool include_space);
DLLEXPORT int get_last_leader_offset(char *line, char **flags);
