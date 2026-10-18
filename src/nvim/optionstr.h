#pragma once

#include <stdint.h>  // IWYU pragma: keep

#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

typedef enum {
  kFillchars,
  kListchars,
} CharsOption;

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void didset_string_options(void);
DLLEXPORT void check_buf_options(buf_T *buf);
DLLEXPORT void free_string_option(char *p);
DLLEXPORT void clear_string_option(char **pp);
DLLEXPORT void check_string_option(char **pp);
DLLEXPORT int check_signcolumn(char *scl, win_T *wp);
DLLEXPORT const char *check_stl_option(char *s, const CharBuf *errbuf);
DLLEXPORT bool check_illegal_path_names(char *val, uint32_t flags);
DLLEXPORT int expand_set_str_generic(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *did_set_str_generic(optset_T *args);
DLLEXPORT const char *validate_str_generic(optset_T *args);
DLLEXPORT const char *did_set_ambiwidth(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *did_set_emoji(optset_T *args);
DLLEXPORT const char *did_set_background(optset_T *args);
DLLEXPORT const char *validate_backspace(optset_T *args);
DLLEXPORT const char *validate_backupcopy(optset_T *args);
DLLEXPORT const char *did_set_backupcopy(optset_T *args);
DLLEXPORT const char *validate_backupext_or_patchmode(optset_T *args);
DLLEXPORT const char *did_set_breakat(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *did_set_breakindentopt(optset_T *args);
DLLEXPORT const char *validate_buftype(optset_T *args);
DLLEXPORT const char *did_set_buftype(optset_T *args);
DLLEXPORT const char *validate_chars_option(optset_T *args);
DLLEXPORT const char *did_set_chars_option(optset_T *args);
DLLEXPORT int expand_set_chars_option(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *did_set_cinoptions(optset_T *args);
DLLEXPORT const char *validate_colorcolumn(optset_T *args);
DLLEXPORT const char *did_set_colorcolumn(optset_T *args);
DLLEXPORT const char *validate_comments(optset_T *args);
DLLEXPORT const char *validate_commentstring(optset_T *args);
DLLEXPORT const char *validate_complete(optset_T *args);
DLLEXPORT const char *did_set_complete(optset_T *args);
DLLEXPORT const char *validate_completeitemalign(optset_T *args);
DLLEXPORT const char *did_set_completeitemalign(optset_T *args);
DLLEXPORT const char *did_set_completeopt(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *did_set_completeslash(optset_T *args);
DLLEXPORT const char *validate_concealcursor(optset_T *args);
DLLEXPORT int expand_set_concealcursor(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *validate_cpoptions(optset_T *args);
DLLEXPORT int expand_set_cpoptions(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *validate_cursorlineopt(optset_T *args);
DLLEXPORT const char *did_set_cursorlineopt(optset_T *args);
DLLEXPORT const char *did_set_diffanchors(optset_T *args);
DLLEXPORT const char *did_set_diffopt(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT int expand_set_diffopt(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *did_set_display(optset_T *args);
DLLEXPORT const char *validate_encoding(optset_T *args);
DLLEXPORT const char *did_set_encoding(optset_T *args);
DLLEXPORT int expand_set_encoding(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *validate_eventignore(optset_T *args);
DLLEXPORT int expand_set_eventignore(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *validate_fileformat(optset_T *args);
DLLEXPORT const char *did_set_fileformat(optset_T *args);
DLLEXPORT char *get_fileformat_name(expand_T *xp FUNC_ATTR_UNUSED, int idx);
DLLEXPORT const char *validate_filetype(optset_T *args);
DLLEXPORT const char *did_set_filetype_or_syntax(optset_T *args);
DLLEXPORT const char *did_set_foldexpr(optset_T *args);
DLLEXPORT const char *did_set_foldignore(optset_T *args);
DLLEXPORT const char *validate_foldmarker(optset_T *args);
DLLEXPORT const char *did_set_foldmarker(optset_T *args);
DLLEXPORT const char *did_set_foldmethod(optset_T *args);
DLLEXPORT const char *validate_formatoptions(optset_T *args);
DLLEXPORT int expand_set_formatoptions(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *did_set_guicursor(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *did_set_helpfile(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *validate_helplang(optset_T *args);
DLLEXPORT const char *validate_highlight(optset_T *args);
DLLEXPORT const char *did_set_iconstring(optset_T *args);
DLLEXPORT const char *did_set_inccommand(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *validate_isopt(optset_T *args);
DLLEXPORT const char *did_set_iskeyword(optset_T *args);
DLLEXPORT const char *did_set_isopt(optset_T *args);
DLLEXPORT const char *did_set_keymap(optset_T *args);
DLLEXPORT const char *did_set_keymodel(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *validate_lispoptions(optset_T *args);
DLLEXPORT const char *validate_matchpairs(optset_T *args);
DLLEXPORT const char *validate_mkspellmem(optset_T *args);
DLLEXPORT const char *validate_mouse(optset_T *args);
DLLEXPORT int expand_set_mouse(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *validate_mousescroll(optset_T *args);
DLLEXPORT const char *did_set_mousescroll(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *did_set_rulerformat(optset_T *args);
DLLEXPORT const char *did_set_selection(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *validate_sessionoptions(optset_T *args);
DLLEXPORT const char *validate_shada(optset_T *args);
DLLEXPORT const char *validate_shellpipe_redir(optset_T *args);
DLLEXPORT int expand_set_shortmess(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *validate_shortmess(optset_T *args);
DLLEXPORT const char *validate_showbreak(optset_T *args);
DLLEXPORT const char *did_set_showcmdloc(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *validate_signcolumn(optset_T *args);
DLLEXPORT const char *did_set_signcolumn(optset_T *args);
DLLEXPORT const char *validate_spellcapcheck(optset_T *args);
DLLEXPORT const char *did_set_spellcapcheck(optset_T *args);
DLLEXPORT const char *validate_spellfile(optset_T *args);
DLLEXPORT const char *did_set_spellfile(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *validate_spelllang(optset_T *args);
DLLEXPORT const char *did_set_spelllang(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *did_set_spelloptions(optset_T *args);
DLLEXPORT const char *validate_spellsuggest(optset_T *args);
DLLEXPORT const char *did_set_spellsuggest(optset_T *args);
DLLEXPORT const char *did_set_splitkeep(optset_T *args FUNC_ATTR_UNUSED);
DLLEXPORT const char *validate_statustabline_rulerformat(optset_T *args);
DLLEXPORT const char *did_set_statuscolumn(optset_T *args);
DLLEXPORT const char *did_set_statusline(optset_T *args);
DLLEXPORT const char *did_set_tabline(optset_T *args);
DLLEXPORT const char *did_set_tagcase(optset_T *args);
DLLEXPORT const char *did_set_titlestring(optset_T *args);
DLLEXPORT const char *validate_vartabstop(optset_T *args);
DLLEXPORT const char *did_set_vartabstop(optset_T *args);
DLLEXPORT const char *did_set_verbosefile(optset_T *args);
DLLEXPORT const char *did_set_virtualedit(optset_T *args);
DLLEXPORT const char *validate_whichwrap(optset_T *args);
DLLEXPORT int expand_set_whichwrap(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *validate_wildmode(optset_T *args);
DLLEXPORT const char *did_set_wildmode(optset_T *args);
DLLEXPORT const char *did_set_winbar(optset_T *args);
DLLEXPORT const char *validate_border(optset_T *args);
DLLEXPORT const char *did_set_winhighlight(optset_T *args);
DLLEXPORT int expand_set_winhighlight(optexpand_T *args, int *numMatches, char ***matches);
DLLEXPORT const char *opt_strings_check(const char *val, const OptSchemaItem *schema,
                                        const CharBuf *errbuf) FUNC_ATTR_NONNULL_ARG(1, 2);
DLLEXPORT void opt_fill(const char *value, FieldHashfn get_field, void *out) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void *opt_keyset(const char *value, OptIndex opt_idx, void *keyset);
DLLEXPORT int check_ff_value(char *p);
DLLEXPORT const char *set_chars_option(win_T *wp, const char *value, CharsOption what, bool apply,
                                       const CharBuf *errbuf);
DLLEXPORT char *get_fillchars_name(expand_T *xp FUNC_ATTR_UNUSED, int idx);
DLLEXPORT char *get_listchars_name(expand_T *xp FUNC_ATTR_UNUSED, int idx);
DLLEXPORT const char *check_chars_options(void);
DLLEXPORT const char *validate_previewpopup(optset_T *args);
DLLEXPORT int expand_set_popupoption(optexpand_T *args, int *numMatches, char ***matches);
