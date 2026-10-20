#pragma once

#include <stdio.h>  // IWYU pragma: keep

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/api/private/helpers.h"
#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/option_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// flags for buf_copy_options()
enum {
  BCO_ENTER  = 1,  ///< going to enter the buffer
  BCO_ALWAYS = 2,  ///< always copy the options
  BCO_NOHELP = 4,  ///< don't touch the help related options
};

/// Flags for option-setting functions
///
/// When OPT_GLOBAL and OPT_LOCAL are both missing, set both local and global
/// values, get local value.
typedef enum {
  OPT_GLOBAL    = 0x01,  ///< Use global value.
  OPT_LOCAL     = 0x02,  ///< Use local value.
  OPT_MODELINE  = 0x04,  ///< Option in modeline.
  OPT_WINONLY   = 0x08,  ///< Only set window-local options.
  OPT_NOWIN     = 0x10,  ///< Don’t set window-local options.
  OPT_ONECOLUMN = 0x20,  ///< list options one per line
  OPT_NO_REDRAW = 0x40,  ///< ignore redraw flags on option
  OPT_SKIPRTP   = 0x80,  ///< "skiprtp" in 'sessionoptions'
} OptionSetFlags;

/// :set boolean option prefix
typedef enum {
  PREFIX_NO = 0,  ///< "no" prefix
  PREFIX_NONE,    ///< no prefix
  PREFIX_INV,     ///< "inv" prefix
} set_prefix_T;

/// Gets the option-domain name of an option's type.
static inline const char *optval_type_name(const ObjectType type)
{
  switch (type) {
  case kObjectTypeBoolean:
    return "boolean";
  case kObjectTypeInteger:
    return "number";
  case kObjectTypeString:
    return "string";
  default:
    abort();
  }
}

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

DLLEXPORT void set_init_tablocal(void);
DLLEXPORT void set_init_1(bool clean_arg);
DLLEXPORT Object get_option_default(const OptIndex opt_idx, int opt_flags);
DLLEXPORT void free_all_options(void);
DLLEXPORT void set_init_2(bool headless);
DLLEXPORT void set_init_3(void);
DLLEXPORT void set_helplang_default(const char *lang);
DLLEXPORT void set_title_defaults(void);
DLLEXPORT void ex_set(exarg_T *eap);
DLLEXPORT const char *find_option_end(const char *arg, OptIndex *opt_idxp);
DLLEXPORT Object get_option_newval(OptIndex opt_idx, int opt_flags, set_prefix_T prefix,
                                   char **argp, int nextchar, set_op_T op, void *varp,
                                   Object *oldval_override, const char **errmsg)
  FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int do_set(char *arg, int opt_flags);
DLLEXPORT int string_to_key(char *arg);
DLLEXPORT void did_set_title(void);
DLLEXPORT void set_options_bin(int oldval, int newval, int opt_flags);
DLLEXPORT void check_options(void);
DLLEXPORT int was_set_insecurely(win_T *const wp, OptIndex opt_idx, int opt_flags);
DLLEXPORT uint32_t *insecure_flag(win_T *const wp, OptIndex opt_idx, int opt_flags);
DLLEXPORT void redraw_titles(void);
DLLEXPORT bool valid_name(const char *val, const char *allowed)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void check_blending(win_T *wp);
DLLEXPORT bool parse_winhl_opt(const char *winhl, win_T *wp);
DLLEXPORT sctx_T *get_option_sctx(OptIndex opt_idx);
DLLEXPORT void set_option_sctx(OptIndex opt_idx, int opt_flags, sctx_T script_ctx);
DLLEXPORT void apply_optionset_autocmd_now(OptIndex opt_idx, int opt_flags, Object oldval,
                                           Object oldval_g, Object oldval_l, Object newval);
DLLEXPORT const char *did_set_global_undolevels(OptInt value, OptInt old_value);
DLLEXPORT const char *did_set_buflocal_undolevels(buf_T *buf, OptInt value, OptInt old_value);
DLLEXPORT void check_redraw_for(buf_T *buf, win_T *win, uint32_t flags);
DLLEXPORT void check_redraw(uint32_t flags);
DLLEXPORT bool is_tty_option(const char *name)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT Object get_tty_option(const char *name);
DLLEXPORT bool set_tty_option(const char *name, char *value);
DLLEXPORT OptIndex find_option(const char *const name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool is_dict_option(OptIndex opt_idx);
DLLEXPORT void optval_free_read(OptIndex opt_idx, Object value);
DLLEXPORT void optval_free(Object o);
DLLEXPORT bool option_equal(Object o1, Object o2);
DLLEXPORT Object opt_from_tristate(TriState t);
DLLEXPORT Object optval_own(OptIndex opt_idx, Object value);
DLLEXPORT Object opt_from_varp(OptIndex opt_idx, void *varp) FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT Object optval_from_obj(OptIndex opt_idx, Object o, set_op_T op, bool *error);
DLLEXPORT bool is_option_hidden(OptIndex opt_idx);
DLLEXPORT bool option_has_type(OptIndex opt_idx, ObjectType type);
DLLEXPORT bool option_has_scope(OptIndex opt_idx, OptScope scope);
DLLEXPORT ssize_t option_scope_idx(OptIndex opt_idx, OptScope scope);
DLLEXPORT uint32_t get_option_flags(OptIndex opt_idx);
DLLEXPORT Object get_option_value(OptIndex opt_idx, int opt_flags);
DLLEXPORT const char *validate_and_prepare_option_value(optset_T *args);
DLLEXPORT void free_option_prepared(optset_T *args);
DLLEXPORT void set_option_direct(OptIndex opt_idx, Object value, int opt_flags, scid_T set_sid);
DLLEXPORT void set_option_direct_for(OptIndex opt_idx, Object value, int opt_flags, scid_T set_sid,
                                     OptScope scope, void *const from);
DLLEXPORT const char *set_option_value(const OptIndex opt_idx, const Object value, int opt_flags,
                                       const bool value_replaced, const CharBuf *errbuf);
DLLEXPORT void set_option_value_give_err(const OptIndex opt_idx, Object value, int opt_flags);
DLLEXPORT Object get_option_value_for(OptIndex opt_idx, int opt_flags, const OptScope scope,
                                      void *const from, Error *err);
DLLEXPORT void set_option_value_for(OptIndex opt_idx, Object value, const int opt_flags,
                                    const OptScope scope, void *const from,
                                    const bool value_replaced, Error *err);
DLLEXPORT void ui_refresh_options(void);
DLLEXPORT int makeset(FILE *fd, int opt_flags, int local_only);
DLLEXPORT int makefoldset(FILE *fd);
DLLEXPORT void *get_varp_scope_from(vimoption_T *p, int opt_flags, buf_T *buf, win_T *win);
DLLEXPORT void *get_varp_scope(vimoption_T *p, int opt_flags);
DLLEXPORT void *get_option_varp_scope_from(OptIndex opt_idx, int opt_flags, buf_T *buf, win_T *win);
DLLEXPORT void *get_varp_from(vimoption_T *p, buf_T *buf, win_T *win);
DLLEXPORT char *get_equalprg(void);
DLLEXPORT Callback *get_findfunc(void);
DLLEXPORT void win_copy_options(win_T *wp_from, win_T *wp_to);
DLLEXPORT void copy_winopt(winopt_T *from, winopt_T *to);
DLLEXPORT void clear_winopt(winopt_T *wop);
DLLEXPORT void didset_window_options(win_T *wp, bool valid_cursor);
DLLEXPORT void buf_copy_options(buf_T *buf, int flags);
DLLEXPORT void reset_modifiable(void);
DLLEXPORT void set_iminsert_global(buf_T *buf);
DLLEXPORT void set_imsearch_global(buf_T *buf);
DLLEXPORT void set_context_in_set_cmd(expand_T *xp, char *arg, int opt_flags);
DLLEXPORT int ExpandSettings(expand_T *xp, regmatch_T *regmatch, char *fuzzystr, int *numMatches,
                             char ***matches, const bool can_fuzzy);
DLLEXPORT char *escape_option_str_cmdline(char *var);
DLLEXPORT int ExpandOldSetting(int *numMatches, char ***matches);
DLLEXPORT int ExpandStringSetting(expand_T *xp, regmatch_T *regmatch, int *numMatches,
                                  char ***matches);
DLLEXPORT int ExpandSettingSubtract(expand_T *xp, regmatch_T *regmatch, int *numMatches,
                                    char ***matches);
DLLEXPORT bool shortmess(int x);
DLLEXPORT void vimrc_found(char *fname, char *envname);
DLLEXPORT bool option_was_set(OptIndex opt_idx);
DLLEXPORT void reset_option_was_set(OptIndex opt_idx);
DLLEXPORT int fill_culopt_flags(char *val, unsigned *flags);
DLLEXPORT int option_set_callback_func(char *optval, Callback *optcb);
DLLEXPORT bool can_bs(int what);
DLLEXPORT unsigned get_bkc_flags(buf_T *buf);
DLLEXPORT char *get_flp_value(buf_T *buf);
DLLEXPORT unsigned get_ve_flags(win_T *wp);
DLLEXPORT char *get_showbreak_value(win_T *const win) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int get_fileformat(const buf_T *buf)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int get_fileformat_force(const buf_T *buf, const exarg_T *eap) FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT int default_fileformat(void);
DLLEXPORT void set_fileformat(int eol_style, int opt_flags);
DLLEXPORT char *skip_to_option_part(const char *p);
DLLEXPORT size_t copy_option_part(char **option, char *buf, size_t maxlen, char *sep_chars);
DLLEXPORT bool option_next_keyval(const char **p, const char **key, size_t *keylen,
                                  const char **val, size_t *vallen) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool option_slice_eq(const char *slice, size_t slicelen, const char *name)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int csh_like_shell(void);
DLLEXPORT bool fish_like_shell(void);
DLLEXPORT dict_T *get_winbuf_options(const int bufopt) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT int64_t get_scrolloff_value(win_T *wp);
DLLEXPORT int64_t get_scrolloffpad_value(win_T *wp);
DLLEXPORT int64_t get_sidescrolloff_value(win_T *wp);
DLLEXPORT Dict get_option_info(String name, int opt_flags, buf_T *buf, win_T *win, Arena *arena,
                               Error *err);
DLLEXPORT Dict get_all_options_info(Arena *arena);
DLLEXPORT OptIndex find_option_len(const char *const name, size_t len)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT vimoption_T *get_option(OptIndex opt_idx);

#include "nvim/func_attr.h"
