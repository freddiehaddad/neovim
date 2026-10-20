#pragma once

#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep
#include "nvim/undo_defs.h"  // IWYU pragma: keep

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT int u_save_cursor(void);
DLLEXPORT int u_save(linenr_T top, linenr_T bot);
DLLEXPORT int u_save_buf(buf_T *buf, linenr_T top, linenr_T bot);
DLLEXPORT int u_savesub(linenr_T lnum);
DLLEXPORT int u_inssub(linenr_T lnum);
DLLEXPORT int u_savedel(linenr_T lnum, linenr_T nlines);
DLLEXPORT bool undo_allowed(buf_T *buf);
DLLEXPORT int u_savecommon(buf_T *buf, linenr_T top, linenr_T bot, linenr_T newbot, bool reload);
DLLEXPORT void u_compute_hash(buf_T *buf, uint8_t *hash);
DLLEXPORT char *u_get_undo_file_name(const char *const buf_ffname, const bool reading)
  FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void u_write_undo(const char *const name, const bool forceit, buf_T *const buf,
                            uint8_t *const hash) FUNC_ATTR_NONNULL_ARG(3, 4);
DLLEXPORT void u_read_undo(char *name, const uint8_t *hash, const char *orig_name FUNC_ATTR_UNUSED)
  FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT void u_undo(int count);
DLLEXPORT void u_redo(int count);
DLLEXPORT bool u_undo_and_forget(int count, bool do_buf_event);
DLLEXPORT void undo_time(int step, bool sec, bool file, bool absolute);
DLLEXPORT void undo_fmt_time(char *buf, size_t buflen, time_t tt);
DLLEXPORT void u_sync(bool force);
DLLEXPORT void ex_undolist(exarg_T *eap);
DLLEXPORT void ex_undojoin(exarg_T *eap);
DLLEXPORT void u_unchanged(buf_T *buf);
DLLEXPORT void u_find_first_changed(void);
DLLEXPORT void u_update_save_nr(buf_T *buf);
DLLEXPORT void u_forget_unchanged(buf_T *buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void u_clearall(buf_T *buf);
DLLEXPORT void u_blockfree(buf_T *buf);
DLLEXPORT void u_clearallandblockfree(buf_T *buf);
DLLEXPORT void u_clearline(buf_T *buf);
DLLEXPORT void u_undoline(void);
DLLEXPORT bool bufIsChanged(buf_T *buf) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool anyBufIsChanged(void) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool curbufIsChanged(void) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void f_undofile(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_undotree(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void u_update_named_mark(buf_T *buf, int idx);
DLLEXPORT u_header_T *u_force_get_undo_header(buf_T *buf);
DLLEXPORT void u_checkpoint(UndoCheckpoint *uc, buf_T *buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void u_rollback(UndoCheckpoint *uc, buf_T *buf) FUNC_ATTR_NONNULL_ALL;
