#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "nvim/api/private/defs.h"
#include "nvim/autocmd_defs.h"
#include "nvim/buffer_defs.h"
#include "nvim/func_attr.h"
#include "nvim/pos_defs.h"
#include "nvim/types_defs.h"

/// Values for in_cinkeys()
enum {
  KEY_OPEN_FORW = 0x101,
  KEY_OPEN_BACK = 0x102,
  KEY_COMPLETE  = 0x103,  ///< end of completion
};

/// Values for change_indent()
enum {
  INDENT_SET = 1,  ///< set indent
  INDENT_INC = 2,  ///< increase indent
  INDENT_DEC = 3,  ///< decrease indent
};

/// flags for beginline()
enum {
  BL_WHITE = 1,  ///< cursor on first non-white in the line
  BL_SOL   = 2,  ///< use 'sol' option
  BL_FIX   = 4,  ///< don't leave cursor on a NUL
};

/// flags for insertchar()
enum {
  INSCHAR_FORMAT   = 1,   ///< force formatting
  INSCHAR_DO_COM   = 2,   ///< format comments
  INSCHAR_CTRLV    = 4,   ///< char typed just after CTRL-V
  INSCHAR_NO_FEX   = 8,   ///< don't use 'formatexpr'
  INSCHAR_COM_LIST = 16,  ///< format comments with list/2nd line indent
};

bool edit(int cmdchar, bool startln, int count);
void ins_redraw(bool ready);
void edit_putchar(int c, bool highlight);
char *buf_prompt_text(const buf_T *buf) FUNC_ATTR_NONNULL_ALL FUNC_ATTR_WARN_UNUSED_RESULT
  FUNC_ATTR_PURE;
char *prompt_text(void) FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
bool prompt_curpos_editable(void) FUNC_ATTR_PURE;
void edit_unputchar(void);
void display_dollar(colnr_T col_arg);
void undisplay_dollar(void);
void truncate_spaces(char *line, size_t len);
void backspace_until_column(int col);
int get_literal(bool no_simplify);
void insertchar(int c, int flags, int second_indent);
void start_arrow(pos_T *end_insert_pos, bool end_change, int move_key);
int stop_arrow(void);
void set_last_insert(int c);
void free_last_insert(void);
void beginline(int flags);
int oneright(void);
int oneleft(void);
void cursor_up_inner(win_T *wp, linenr_T n, bool skip_conceal);
int cursor_up(linenr_T n, bool upd_topline);
void cursor_down_inner(win_T *wp, int n, bool skip_conceal);
int cursor_down(int n, bool upd_topline);
int stuff_inserted(int c, int count, int no_esc);
String get_last_insert(void) FUNC_ATTR_PURE;
char *get_last_insert_save(void);
void replace_push(char *str, size_t len);
void replace_push_nul(void);
void replace_join(int off);
bool ins_eol(int c);
int ins_copychar(linenr_T lnum);
colnr_T get_nolist_virtcol(void);
bool get_can_cindent(void);
void set_can_cindent(bool val);
int ins_apply_autocmds(event_T event);
