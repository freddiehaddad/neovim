#pragma once

#include <stdint.h>

#include "nvim/buffer_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/gettext_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/marktree_defs.h"
#include "nvim/types_defs.h"

/// Values for buflist_getfile()
enum getf_values {
  GETF_SETMARK = 0x01,  ///< set pcmark before jumping
  GETF_ALT     = 0x02,  ///< jumping to alternate file (not buf num)
  GETF_SWITCH  = 0x04,  ///< respect 'switchbuf' settings when jumping
};

// Return values of getfile()
enum getf_retvalues {
  GETFILE_ERROR       = 1,   ///< normal error
  GETFILE_NOT_WRITTEN = 2,   ///< "not written" error
  GETFILE_SAME_FILE   = 0,   ///< success, same file
  GETFILE_OPEN_OTHER  = -1,  ///< success, opened another file
  GETFILE_UNUSED      = 8,
};

/// Values for buflist_new() flags
enum bln_values {
  BLN_CURBUF = 1,   ///< May re-use curbuf for new buffer
  BLN_LISTED = 2,   ///< Put new buffer in buffer list
  BLN_DUMMY  = 4,   ///< Allocating dummy buffer
  BLN_NEW    = 8,   ///< create a new buffer
  BLN_NOOPT  = 16,  ///< Don't copy options to existing buffer
  // BLN_DUMMY_OK = 32,  // also find an existing dummy buffer
  // BLN_REUSE = 64,   // may re-use number from buf_reuse
  BLN_NOCURWIN = 128,  ///< buffer is not associated with curwin
};

/// Values for action argument for do_buffer_ext() and close_buffer()
enum dobuf_action_values {
  DOBUF_GOTO   = 0,  ///< go to specified buffer
  DOBUF_SPLIT  = 1,  ///< split window and go to specified buffer
  DOBUF_UNLOAD = 2,  ///< unload specified buffer(s)
  DOBUF_DEL    = 3,  ///< delete specified buffer(s) from buflist
  DOBUF_WIPE   = 4,  ///< delete specified buffer(s) really
};

/// Values for start argument for do_buffer_ext()
enum dobuf_start_values {
  DOBUF_CURRENT = 0,  ///< "count" buffer from current buffer
  DOBUF_FIRST   = 1,  ///< "count" buffer from first buffer
  DOBUF_LAST    = 2,  ///< "count" buffer from last buffer
  DOBUF_MOD     = 3,  ///< "count" mod. buffer from current buffer
};

/// Values for flags argument of do_buffer_ext()
enum dobuf_flags_value {
  DOBUF_FORCEIT  = 1,  ///< :cmd!
  DOBUF_SKIPHELP = 4,  ///< skip or keep help buffers depending on b_help of the
                       ///< starting buffer
};

/// flags for buf_freeall()
enum bfa_values {
  BFA_DEL          = 1,  ///< buffer is going to be deleted
  BFA_WIPE         = 2,  ///< buffer is going to be wiped out
  BFA_KEEP_UNDO    = 4,  ///< do not free undo information
  BFA_IGNORE_ABORT = 8,  ///< do not abort for aborting()
};

EXTERN char *msg_loclist INIT( = N_("[Location List]"));
EXTERN char *msg_qflist INIT( = N_("[Quickfix List]"));

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT int calc_percentage(int64_t part, int64_t whole);
DLLEXPORT int get_highest_fnum(void);
DLLEXPORT bool buf_ensure_loaded(buf_T *buf);
DLLEXPORT int open_buffer(bool read_stdin, exarg_T *eap, int flags_arg);
DLLEXPORT void set_bufref(bufref_T *bufref, buf_T *buf);
DLLEXPORT bool bufref_valid(bufref_T *bufref) FUNC_ATTR_PURE;
DLLEXPORT bool buf_valid(buf_T *buf) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void buf_close_terminal(buf_T *buf);
DLLEXPORT bool close_buffer(win_T *win, buf_T *buf, int action, bool abort_if_last,
                            bool ignore_abort, bool set_context)
  FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT void buf_clear_file(buf_T *buf);
DLLEXPORT void buf_clear(void);
DLLEXPORT bool buf_freeall(buf_T *buf, int flags) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void goto_buffer(exarg_T *eap, int start, int dir, int count);
DLLEXPORT void handle_swap_exists(bufref_T *old_curbuf);
DLLEXPORT char *do_bufdel(int command, char *arg, int addr_count, int start_bnr, int end_bnr,
                          int forceit);
DLLEXPORT int do_buffer(int action, int start, int dir, int count, int forceit);
DLLEXPORT void set_curbuf(buf_T *buf, int action, bool update_jumplist);
DLLEXPORT void do_autochdir(void);
DLLEXPORT void no_write_message_buf(buf_T *buf);
DLLEXPORT void no_write_message(void);
DLLEXPORT void no_write_message_nobang(const buf_T *const buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT buf_T *buflist_new(char *ffname_arg, char *sfname_arg, linenr_T lnum, int flags);
DLLEXPORT bool curbuf_reusable(void);
DLLEXPORT void free_buf_options(buf_T *buf, bool free_p_ff);
DLLEXPORT int buflist_getfile(int n, linenr_T lnum, int options, int forceit);
DLLEXPORT buf_T *buflist_findname_exp(char *fname);
DLLEXPORT buf_T *buflist_findname(char *ffname);
DLLEXPORT int buflist_findpat(const char *pattern, const char *pattern_end, bool unlisted,
                              bool diffmode, bool curtab_only)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int ExpandBufnames(char *pat, int *num_file, char ***file, int options);
DLLEXPORT buf_T *buflist_findnr(int nr);
DLLEXPORT char *buflist_nr2name(int n, int fullname, int helptail);
DLLEXPORT void buflist_setfpos(buf_T *const buf, win_T *const win, linenr_T lnum, colnr_T col,
                               bool copy_options)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void get_winopts(buf_T *buf);
DLLEXPORT fmark_T *buflist_findfmark(buf_T *buf) FUNC_ATTR_PURE;
DLLEXPORT linenr_T buflist_findlnum(buf_T *buf) FUNC_ATTR_PURE;
DLLEXPORT void buflist_list(exarg_T *eap);
DLLEXPORT int buflist_name_nr(int fnum, char **fname, linenr_T *lnum);
DLLEXPORT int setfname(buf_T *buf, char *ffname_arg, char *sfname_arg, bool message);
DLLEXPORT void buf_set_name(int fnum, char *name);
DLLEXPORT void buf_name_changed(buf_T *buf);
DLLEXPORT buf_T *setaltfname(char *ffname, char *sfname, linenr_T lnum);
DLLEXPORT char *getaltfname(bool errmsg);
DLLEXPORT int buflist_add(char *fname, int flags);
DLLEXPORT void buflist_altfpos(win_T *win);
DLLEXPORT bool otherfile(char *ffname)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void buf_set_file_id(buf_T *buf);
DLLEXPORT void fileinfo(int fullname, int shorthelp, bool dont_truncate);
DLLEXPORT int col_print(char *buf, size_t buflen, int col, int vcol);
DLLEXPORT void maketitle(void);
DLLEXPORT void resettitle(void);
DLLEXPORT void free_titles(void);
DLLEXPORT int get_rel_pos(win_T *wp, char *buf, int buflen);
DLLEXPORT int append_arg_number(win_T *wp, char *buf, size_t buflen) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void fname_expand(buf_T *buf, char **ffname, char **sfname);
DLLEXPORT bool bt_prompt(buf_T *buf) FUNC_ATTR_PURE;
DLLEXPORT bool bt_cmdwin(const buf_T *buf) FUNC_ATTR_PURE;
DLLEXPORT void ex_buffer_all(exarg_T *eap);
DLLEXPORT void do_modelines(int flags);
DLLEXPORT bool bt_help(const buf_T *const buf) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool bt_normal(const buf_T *const buf) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool bt_quickfix(const buf_T *const buf) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool bt_terminal(const buf_T *const buf) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool bt_nofilename(const buf_T *const buf) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool bt_nofile(const buf_T *const buf) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool bt_dontwrite(const buf_T *const buf) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool bt_dontwrite_msg(const buf_T *const buf)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool buf_hide(const buf_T *const buf) FUNC_ATTR_PURE;
DLLEXPORT char *buf_spname(buf_T *buf);
DLLEXPORT char *buf_get_fname(const buf_T *buf)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void set_buflisted(int on);
DLLEXPORT bool buf_contents_changed(buf_T *buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void wipe_buffer(buf_T *buf, bool aucmd);
DLLEXPORT bool buf_is_empty(buf_T *buf);
DLLEXPORT void buf_inc_changedtick(buf_T *const buf) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void buf_set_changedtick(buf_T *const buf, const varnumber_T changedtick)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void read_buffer_into(buf_T *buf, linenr_T start, linenr_T end, StringBuilder *sb)
  FUNC_ATTR_NONNULL_ALL;

#include "buffer.h.inline.generated.h"

/// Get b:changedtick value
///
/// Faster then querying b:.
///
/// @param[in]  buf  Buffer to get b:changedtick from.
static inline varnumber_T buf_get_changedtick(const buf_T *const buf)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_ALWAYS_INLINE FUNC_ATTR_PURE
  FUNC_ATTR_WARN_UNUSED_RESULT
{
  return buf->changedtick_di.di_tv.vval.v_number;
}

static inline uint32_t buf_meta_total(const buf_T *b, MetaIndex m)
{
  return b->b_marktree->meta_root[m];
}
