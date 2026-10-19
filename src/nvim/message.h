#pragma once

#include <errno.h>
#include <stdbool.h>
#include <stddef.h>  // IWYU pragma: keep

#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/grid_defs.h"
#include "nvim/keycodes_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/message_defs.h"  // IWYU pragma: keep
#include "nvim/option_defs.h"  // IWYU pragma: keep

/// Types of dialogs passed to do_dialog().
enum {
  VIM_GENERIC   = 0,
  VIM_ERROR     = 1,
  VIM_WARNING   = 2,
  VIM_INFO      = 3,
  VIM_QUESTION  = 4,
  VIM_LAST_TYPE = 4,  ///< sentinel value
};

/// Return values for functions like vim_dialogyesno()
enum {
  VIM_YES        = 2,
  VIM_NO         = 3,
  VIM_CANCEL     = 4,
  VIM_ALL        = 5,
  VIM_DISCARDALL = 6,
};

extern MessageHistoryEntry *msg_hist_last;

EXTERN bool msg_ext_need_clear INIT( = false);
/// Set to true to force grouping a set of message chunks into a single `cmdline_show` event.
EXTERN bool msg_ext_skip_flush INIT( = false);
/// Set to true when previous message should be overwritten.
EXTERN bool msg_ext_overwrite INIT( = false);
/// Set to true to avoid setting "verbose" kind for "last set" messages.
EXTERN bool msg_ext_skip_verbose INIT( = false);
/// Set to false for non-internal messages to determine UI callback |api-fast| context.
EXTERN bool msg_ext_fast INIT( = true);

/// allocated grid for messages. Used unless ext_messages is active.
/// See also the description at msg_scroll_flush()
EXTERN ScreenGrid msg_grid INIT( = SCREEN_GRID_INIT);
EXTERN int msg_grid_pos INIT( = 0);

/// "adjusted" message grid. This grid accepts positions relative to
/// default_grid. Internally it will be translated to a position on msg_grid
/// relative to the start of the message area, or directly mapped to default_grid
/// for legacy (display-=msgsep) message scroll behavior.
/// TODO(bfredl): refactor "internal" message logic, msg_row etc
/// to use the correct positions already.
EXTERN GridView msg_grid_adj INIT( = { 0 });

/// value of msg_scrolled at latest msg_scroll_flush.
EXTERN int msg_scrolled_at_flush INIT( = 0);

EXTERN int msg_grid_scroll_discount INIT( = 0);

EXTERN int msg_listdo_overwrite INIT( = 0);

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

DLLEXPORT bool msg_id_exists(int64_t id);
DLLEXPORT void msg_grid_set_pos(int row, bool scrolled);
DLLEXPORT bool msg_use_grid(void);
DLLEXPORT void msg_grid_validate(void);
DLLEXPORT int verb_msg(const char *s);
DLLEXPORT bool msg(const char *s, const int hl_id) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void msg_multiline(String str, int hl_id, bool check_int, bool hist, bool *need_clear)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT MsgID msg_multihl(MsgID id, HlMessage hl_msg, const char *kind, bool history, bool err,
                            MessageData *msg_data, bool *needs_msg_clear, bool raw_for_lua_print);
DLLEXPORT bool msg_keep(const char *s, int hl_id, bool keep, bool multiline) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *msg_strtrunc(const char *s, int force);
DLLEXPORT void trunc_string(const char *s, char *buf, int room_in, int buflen);
DLLEXPORT void reset_last_sourcing(void);
DLLEXPORT void msg_source(int hl_id);
DLLEXPORT bool emsg_not_now(void);
DLLEXPORT bool emsg_multiline(const char *s, const char *kind, int hl_id, bool multiline);
DLLEXPORT bool emsg(const char *s);
DLLEXPORT void emsg_invreg(int name);
DLLEXPORT void iemsg(const char *s);
DLLEXPORT void internal_error(const char *where);
DLLEXPORT void msg_schedule_semsg_cstr(const char *s);
DLLEXPORT void msg_schedule_semsg_multiline_cstr(const char *s);
DLLEXPORT char *msg_trunc(char *s, bool force, int hl_id);
DLLEXPORT char *msg_may_trunc(bool force, char *s);
DLLEXPORT char *msg_progress(char *s, char *id, char *status, int percent, int hl_id, bool hist,
                             bool trunc, bool err);
DLLEXPORT char *msg_with_id(char *s, char *id, const char *kind, MessageData *data, int hl_id,
                            bool hist, bool trunc, bool err);
DLLEXPORT void hl_msg_free(HlMessage hl_msg);
DLLEXPORT void do_autocmd_progress(MsgID msg_id, HlMessage msg, MessageData *msg_data);
DLLEXPORT void msg_hist_clear(int keep);
DLLEXPORT void msg_hist_clear_temp(void);
DLLEXPORT const char *validate_messagesopt(optset_T *args);
DLLEXPORT const char *did_set_messagesopt(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT void ex_messages(exarg_T *eap) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void msg_end_prompt(void);
DLLEXPORT void wait_return(int redraw);
DLLEXPORT void set_keep_msg(const char *s, int hl_id);
DLLEXPORT bool messaging(void);
DLLEXPORT void msgmore(int n);
DLLEXPORT void msg_ext_set_kind(const char *msg_kind);
DLLEXPORT void msg_ext_set_append(bool append);
DLLEXPORT void msg_ext_set_trigger(const char *trigger);
DLLEXPORT void msg_ext_no_fast(void);
DLLEXPORT void msg_start(void);
DLLEXPORT void msg_starthere(void);
DLLEXPORT void msg_putchar(int c);
DLLEXPORT void msg_putchar_hl(int c, int hl_id);
DLLEXPORT void msg_outnum(int n);
DLLEXPORT void msg_home_replace(const char *fname);
DLLEXPORT int msg_outtrans(const char *str, int hl_id, bool hist);
DLLEXPORT const char *msg_outtrans_one(const char *p, int hl_id, bool hist);
DLLEXPORT int msg_outtrans_len(const char *msgstr, int len, int hl_id, bool hist);
DLLEXPORT void msg_make(const char *arg);
DLLEXPORT int msg_outtrans_special(const char *strstart, bool from, int maxlen);
DLLEXPORT char *str2special_save(const char *const str, const bool replace_spaces,
                                 const TriState replace_others)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_RET;
DLLEXPORT char *str2special_arena(const char *const str, const bool replace_spaces,
                                  const TriState replace_others, Arena *const arena)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_MALLOC FUNC_ATTR_NONNULL_RET;
DLLEXPORT const char *str2special(const char **const sp, const bool replace_spaces,
                                  const TriState replace_others, struct keychord *data)
  FUNC_ATTR_NONNULL_ARG(1) FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_RET;
DLLEXPORT void str2specialbuf(const char *sp, char *buf, size_t len) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void msg_prt_line(const char *s, bool list);
DLLEXPORT void msg_puts(const char *s);
DLLEXPORT void msg_puts_title(const char *s);
DLLEXPORT void msg_outtrans_long(const char *longstr, int hl_id);
DLLEXPORT void msg_puts_hl(const char *const s, const int hl_id, const bool hist);
DLLEXPORT void msg_puts_len(const char *const str, const ptrdiff_t len, int hl_id, bool hist)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void msg_line_flush(void);
DLLEXPORT void msg_cursor_goto(int row, int col);
DLLEXPORT bool message_filtered(const char *msg);
DLLEXPORT int msg_scrollsize(void);
DLLEXPORT bool msg_do_throttle(void);
DLLEXPORT void msg_scroll_up(bool may_throttle, bool zerocmd);
DLLEXPORT void msg_scroll_flush(void);
DLLEXPORT void msg_reset_scroll(void);
DLLEXPORT void msg_ui_refresh(void);
DLLEXPORT void msg_ui_flush(void);
DLLEXPORT void may_clear_sb_text(void);
DLLEXPORT void sb_text_start_cmdline(void);
DLLEXPORT void sb_text_restart_cmdline(void);
DLLEXPORT void sb_text_end_cmdline(void);
DLLEXPORT void clear_sb_text(bool all);
DLLEXPORT void show_sb_text(void);
DLLEXPORT void msg_sb_eol(void);
DLLEXPORT int msg_use_printf(void);
DLLEXPORT void repeat_message(void);
DLLEXPORT void msg_clr_eos(void);
DLLEXPORT void msg_clr_eos_force(void);
DLLEXPORT void msg_clr_cmdline(void);
DLLEXPORT bool msg_end(void);
DLLEXPORT void msg_ext_ui_flush(void);
DLLEXPORT void msg_ext_flush_showmode(void);
DLLEXPORT void msg_check(void);
DLLEXPORT int redirecting(void);
DLLEXPORT void verbose_enter(void);
DLLEXPORT void verbose_leave(void);
DLLEXPORT void verbose_enter_scroll(void);
DLLEXPORT void verbose_leave_scroll(void);
DLLEXPORT void verbose_stop(void);
DLLEXPORT int verbose_open(void);
DLLEXPORT void give_warning(const char *message, bool hl, bool hist) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT void msg_advance(int col);
DLLEXPORT void display_confirm_msg(void);
DLLEXPORT void msg_delay(uint64_t ms, bool ignoreinput);
DLLEXPORT void msg_check_for_delay(bool check_msg_scroll);

#include "nvim/func_attr.h"

#include "message.h.generated.h"

// Prefer using semsg(), because perror() may send the output to the wrong
// destination and mess up the screen.
#define PERROR(msg) (void)semsg("%s: %s", (msg), strerror(errno))
