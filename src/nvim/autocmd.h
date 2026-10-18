#pragma once

#include <stdbool.h>
#include <stddef.h>  // IWYU pragma: keep
#include <stdint.h>  // IWYU pragma: keep

#include "klib/kvec.h"
#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/autocmd_defs.h"  // IWYU pragma: keep
#include "nvim/buffer_defs.h"
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/event/defs.h"
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/types_defs.h"

/// For CursorMoved event
EXTERN win_T *last_cursormoved_win INIT( = NULL);
/// For CursorMoved event, only used when last_cursormoved_win == curwin
EXTERN pos_T last_cursormoved INIT( = { 0, 0, 0 });

EXTERN bool autocmd_busy INIT( = false);     ///< Is apply_autocmds() busy?
EXTERN int autocmd_no_enter INIT( = false);  ///< Buf/WinEnter autocmds disabled
EXTERN int autocmd_no_leave INIT( = false);  ///< Buf/WinLeave autocmds disabled

/// When deleting the current buffer, another one must be loaded.
/// If we know which one is preferred, au_new_curbuf is set to it.
EXTERN bufref_T au_new_curbuf INIT( = { NULL, 0, 0 });

// When deleting a buffer/window and autocmd_busy is true, do not free the
// buffer/window. but link it in the list starting with
// au_pending_free_buf/ap_pending_free_win, using b_next/w_next.
// Free the buffer/window when autocmd_busy is being set to false.
EXTERN buf_T *au_pending_free_buf INIT( = NULL);
EXTERN win_T *au_pending_free_win INIT( = NULL);

EXTERN char *autocmd_fname INIT( = NULL);       ///< fname for <afile> on cmdline
EXTERN bool autocmd_fname_full INIT( = false);  ///< autocmd_fname is full path
EXTERN int autocmd_bufnr INIT( = 0);            ///< fnum for <abuf> on cmdline
EXTERN char *autocmd_match INIT( = NULL);       ///< name for <amatch> on cmdline
EXTERN bool did_cursorhold INIT( = true);       ///< set when CursorHold t'gerd

enum {
  AUGROUP_DEFAULT = -1,  ///< default autocmd group
  AUGROUP_ERROR   = -2,  ///< erroneous autocmd group
  AUGROUP_ALL     = -3,  ///< all autocmd groups
  AUGROUP_DELETED = -4,  ///< all autocmd groups
  // AUGROUP_NS      = -5,  // TODO(tjdevries): Support namespaced based augroups
};

enum { BUFLOCAL_PAT_LEN = 25, };

/// Iterates over all the events for auto commands
#define FOR_ALL_AUEVENTS(event) \
  for (event_T event = (event_T)0; (int)event < (int)NUM_EVENTS; event = (event_T)((int)event + 1))

/// Stores events for execution until a known safe state.
/// This should be the default for all new autocommands.
EXTERN MultiQueue *deferred_events INIT( = NULL);

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

DLLEXPORT void autocmd_init(void);
#ifdef EXITFREE
DLLEXPORT void autocmd_free_all_mem(void);
#endif
DLLEXPORT void aucmd_del_for_event_and_group(event_T event, int group);
DLLEXPORT AutoCmdVec *au_get_autocmds_for_event(event_T event) FUNC_ATTR_PURE;
DLLEXPORT void aubuflocal_remove(buf_T *buf);
DLLEXPORT int augroup_add(const char *name);
DLLEXPORT void augroup_del_by_id(int id);
DLLEXPORT void augroup_del(char *name, int id_for_error, bool stupid_legacy_mode);
DLLEXPORT int augroup_find(const char *name) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT char *augroup_name(int group);
DLLEXPORT bool augroup_exists(const char *name) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void do_augroup(char *arg, bool del_group);
#ifdef EXITFREE
DLLEXPORT void free_all_autocmds(void);
#endif
DLLEXPORT event_T event_name2nr(const char *start, char **end);
DLLEXPORT event_T event_name2nr_str(String str);
DLLEXPORT const char *event_nr2name(event_T event)
  FUNC_ATTR_NONNULL_RET FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_CONST;
DLLEXPORT bool event_ignored(event_T event, char *ei) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int check_ei(char *ei, bool win);
DLLEXPORT char *au_event_disable(char *what);
DLLEXPORT void au_event_restore(char *old_ei);
DLLEXPORT void do_autocmd(exarg_T *eap, char *arg_in, int forceit);
DLLEXPORT void do_all_autocmd_events(const char *pat, bool once, int nested, char *cmd, bool del,
                                     int group);
DLLEXPORT int do_autocmd_event(event_T event, const char *pat, bool once, int nested,
                               const char *cmd, bool del, int group) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int autocmd_register(int64_t id, event_T event, const char *pat, int patlen, int group,
                               bool once, bool nested, char *desc, const char *handler_cmd,
                               Callback *handler_fn);
DLLEXPORT size_t aucmd_span_pattern(const char *pat, const char **start)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int do_doautocmd(char *arg_start, bool do_msg, bool *did_something);
DLLEXPORT void ex_doautoall(exarg_T *eap);
DLLEXPORT bool check_nomodeline(char **argp) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void aucmd_defer(event_T event, char *fname, char *fname_io, int group, buf_T *buf,
                           exarg_T *eap, Object *data);
DLLEXPORT void aucmd_defer_modified(buf_T *buf, bool new_val);
DLLEXPORT bool apply_autocmds(event_T event, char *fname, char *fname_io, bool force, buf_T *buf);
DLLEXPORT bool apply_autocmds_win(event_T event, char *fname, char *fname_io, bool force,
                                  buf_T *buf, win_T *win);
DLLEXPORT bool apply_autocmds_exarg(event_T event, char *fname, char *fname_io, bool force,
                                    buf_T *buf, exarg_T *eap);
DLLEXPORT bool apply_autocmds_retval(event_T event, char *fname, char *fname_io, bool force,
                                     buf_T *buf, int *retval);
DLLEXPORT bool has_event(event_T event) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool trigger_cursorhold(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool apply_autocmds_group(event_T event, char *fname, char *fname_io, bool force,
                                    int group, buf_T *buf, win_T *win, exarg_T *eap, Object *data,
                                    bool with_buf);
DLLEXPORT void do_termresponse_autocmd(String sequence, uint64_t channel_id);
DLLEXPORT void block_autocmds(void);
DLLEXPORT void unblock_autocmds(void);
DLLEXPORT bool is_autocmd_blocked(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT char *getnextac(int c, void *cookie, int indent, bool do_concat);
DLLEXPORT bool has_autocmd(event_T event, char *sfname, buf_T *buf) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT char *expand_get_augroup_name(expand_T *xp, int idx);
DLLEXPORT char *aucmd_set_expand_context(expand_T *xp, char *arg, bool doautocmd);
DLLEXPORT char *expand_get_event_name(expand_T *xp, int idx);
DLLEXPORT char *get_event_name_no_group(expand_T *xp FUNC_ATTR_UNUSED, int idx, bool win);
DLLEXPORT bool autocmd_supported(const char *event) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool au_exists(const char *arg) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool aupat_is_buflocal(const char *pat, int patlen) FUNC_ATTR_PURE;
DLLEXPORT int aupat_get_buflocal_nr(const char *pat, int patlen);
DLLEXPORT void aupat_normalize_buflocal_pat(char *dest, const char *pat, int patlen,
                                            int buflocal_nr);
DLLEXPORT int autocmd_delete_event(int group, event_T event, const char *pat) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool autocmd_delete_id(int64_t id);
DLLEXPORT char *aucmd_handler_to_string(AutoCmd *ac) FUNC_ATTR_PURE;
DLLEXPORT void may_trigger_vim_suspend_resume(bool suspend);
DLLEXPORT void do_autocmd_uienter(uint64_t chanid, bool attached);
DLLEXPORT void do_autocmd_focusgained(bool gained);
DLLEXPORT bool do_filetype_autocmd(buf_T *buf, bool force);

#include "nvim/func_attr.h"
