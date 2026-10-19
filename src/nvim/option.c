// option.c: C generated option table for option.rs

#define IN_OPTION_C
#include <assert.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

#include "auto/config.h"
#include "nvim/assert_defs.h"
#include "nvim/gettext_defs.h"
#include "nvim/globals.h"
#include "nvim/message.h"
#include "nvim/option.h"
#include "nvim/option_defs.h"
#include "nvim/option_vars.h"
#include "nvim/os/os_defs.h"
#include "nvim/vim_defs.h"

#ifdef BACKSLASH_IN_FILENAME
# include "nvim/arglist.h"
#endif

#include "option.c.generated.h"

const char *did_set_arabic(optset_T *args);
const char *did_set_autochdir(optset_T *args);
const char *did_set_binary(optset_T *args);
const char *did_set_buflisted(optset_T *args);
const char *did_set_cmdheight(optset_T *args);
const char *did_set_diff(optset_T *args);
const char *did_set_eof_eol_fixeol_bomb(optset_T *args);
const char *did_set_equalalways(optset_T *args);
const char *did_set_foldenable(optset_T *args);
const char *did_set_foldlevel(optset_T *args);
const char *did_set_foldminlines(optset_T *args);
const char *did_set_foldnestmax(optset_T *args);
const char *did_set_follow(optset_T *args);
const char *did_set_helpheight(optset_T *args);
const char *did_set_hlsearch(optset_T *args);
const char *did_set_ignorecase(optset_T *args);
const char *did_set_iminsert(optset_T *args);
const char *did_set_langnoremap(optset_T *args);
const char *did_set_langremap(optset_T *args);
const char *did_set_laststatus(optset_T *args);
const char *did_set_lines_or_columns(optset_T *args);
const char *did_set_lisp(optset_T *args);
const char *did_set_modifiable(optset_T *args);
const char *did_set_modified(optset_T *args);
const char *did_set_number_relativenumber(optset_T *args);
const char *did_set_numberwidth(optset_T *args);
const char *did_set_paste(optset_T *args);
const char *validate_previewwindow(optset_T *args);
const char *did_set_pumblend(optset_T *args);
const char *did_set_readonly(optset_T *args);
const char *did_set_scrollback(optset_T *args);
const char *did_set_scrollbind(optset_T *args);
const char *did_set_shiftwidth_tabstop(optset_T *args);
const char *did_set_showtabline(optset_T *args);
const char *did_set_smoothscroll(optset_T *args);
const char *did_set_spell(optset_T *args);
const char *did_set_swapfile(optset_T *args);
const char *did_set_textwidth(optset_T *args);
const char *did_set_title_icon(optset_T *args);
const char *did_set_titlelen(optset_T *args);
const char *did_set_undofile(optset_T *args);
const char *did_set_undolevels(optset_T *args);
const char *did_set_updatecount(optset_T *args);
const char *validate_wildchar(optset_T *args);
const char *did_set_winblend(optset_T *args);
const char *did_set_window(optset_T *args);
const char *did_set_winheight(optset_T *args);
const char *did_set_winwidth(optset_T *args);
const char *did_set_wrap(optset_T *args);
const char *did_set_xhistory(optset_T *args);

// options[] is initialized in options.generated.h.
// The options with a NULL variable are 'hidden': a set command for them is
// ignored and they are not printed.

#include "options.generated.h"
#include "options_map.generated.h"

/// Find index for an option. Don't go beyond `len` length.
///
/// @param[in]  name  Option name.
/// @param      len   Option name length.
///
/// @return Option index or kOptInvalid if option was not found.
OptIndex find_option_len(const char *const name, size_t len)
  FUNC_ATTR_NONNULL_ALL
{
  int index = find_option_hash(name, len);
  return index >= 0 ? option_hash_elems[index].opt_idx : kOptInvalid;
}

/// Return information for option at 'opt_idx'
vimoption_T *get_option(OptIndex opt_idx)
{
  assert(opt_idx != kOptInvalid);
  return &options[opt_idx];
}
