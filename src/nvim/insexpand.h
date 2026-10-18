#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "nvim/eval/typval_defs.h"
#include "nvim/func_attr.h"
#include "nvim/option_defs.h"
#include "nvim/pos_defs.h"
#include "nvim/types_defs.h"
#include "nvim/vim_defs.h"

/// Array indexes used for cp_text[].
typedef enum {
  CPT_ABBR,   ///< "abbr"
  CPT_KIND,   ///< "kind"
  CPT_MENU,   ///< "menu"
  CPT_INFO,   ///< "info"
  CPT_COUNT,  ///< Number of entries
} cpitem_T;

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void ins_ctrl_x(void);
DLLEXPORT bool ctrl_x_mode_none(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_normal(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_scroll(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_whole_line(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_files(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_tags(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_path_patterns(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_path_defines(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_dictionary(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_thesaurus(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_cmdline(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_function(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_omni(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_spell(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_eval(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_line_or_eval(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_register(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_not_default(void) FUNC_ATTR_PURE;
DLLEXPORT bool ctrl_x_mode_not_defined_yet(void) FUNC_ATTR_PURE;
DLLEXPORT bool compl_status_adding(void);
DLLEXPORT bool compl_status_sol(void);
DLLEXPORT bool compl_status_local(void);
DLLEXPORT void compl_status_clear(void);
DLLEXPORT bool check_compl_option(bool dict_opt);
DLLEXPORT bool vim_is_ctrl_x_key(int c) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool ins_compl_accept_char(int c) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT bool ins_compl_commit_char(int c);
DLLEXPORT int ins_compl_add_infercase(char *str_arg, int len, bool icase, char *fname,
                                      Direction dir, bool cont_s_ipos, int score)
  FUNC_ATTR_NONNULL_ARG(1);
DLLEXPORT bool ins_compl_is_match_selected(void);
DLLEXPORT bool ins_compl_preinsert_longest(void);
DLLEXPORT char *ins_compl_leader(void);
DLLEXPORT int ins_compl_col_range_attr(linenr_T lnum, int col);
DLLEXPORT bool ins_compl_lnum_in_range(linenr_T lnum);
DLLEXPORT bool ins_compl_has_shown_match(void);
DLLEXPORT bool ins_compl_long_shown_match(void);
DLLEXPORT unsigned get_cot_flags(void);
DLLEXPORT bool pum_wanted(void) FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT void ins_compl_show_pum(void);
DLLEXPORT bool compl_match_curr_select(int selected);
DLLEXPORT char *find_word_start(char *ptr) FUNC_ATTR_PURE;
DLLEXPORT char *find_word_end(char *ptr) FUNC_ATTR_PURE;
DLLEXPORT char *find_line_end(char *ptr);
DLLEXPORT void ins_compl_clear(void);
DLLEXPORT bool ins_compl_active(void) FUNC_ATTR_PURE;
DLLEXPORT bool ins_compl_win_active(win_T *wp);
DLLEXPORT bool ins_compl_used_match(void);
DLLEXPORT void ins_compl_init_get_longest(void);
DLLEXPORT bool ins_compl_interrupted(void);
DLLEXPORT bool ins_compl_enter_selects(void);
DLLEXPORT colnr_T ins_compl_col(void);
DLLEXPORT int ins_compl_len(void);
DLLEXPORT bool ins_compl_has_preinsert(void);
DLLEXPORT bool ins_compl_preinsert_effect(void);
DLLEXPORT int ins_compl_bs(void);
DLLEXPORT bool ins_compl_has_autocomplete(void);
DLLEXPORT void ins_compl_addleader(int c);
DLLEXPORT void ins_compl_addfrommatch(void);
DLLEXPORT bool ins_compl_cancel(void);
DLLEXPORT bool ins_compl_prep(int c);
DLLEXPORT void clear_cpt_callbacks(Callback **callbacks, int count);
DLLEXPORT void set_buflocal_cpt_callbacks(buf_T *buf);
DLLEXPORT int set_cpt_callbacks(optset_T *args);
DLLEXPORT bool set_ref_in_cpt_callbacks(Callback *callbacks, int count, int copyID);
DLLEXPORT bool set_ref_in_insexpand_funcs(int copyID);
DLLEXPORT void f_complete(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_complete_add(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_complete_check(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_complete_info(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void ins_compl_delete(bool new_leader);
DLLEXPORT void ins_compl_insert(bool move_cursor, bool insert_prefix);
DLLEXPORT void ins_compl_check_keys(int frequency, bool in_compl_func);
DLLEXPORT int ins_complete(int c, bool enable_pum);
DLLEXPORT void ins_compl_enable_autocomplete(void);
DLLEXPORT void ins_compl_disable_autocomplete(void);
DLLEXPORT void ins_compl_arm_autostart(void);
DLLEXPORT void ins_compl_disarm_autostart(void);
DLLEXPORT bool ins_compl_arm_autocomplete_delay(void);
DLLEXPORT void ins_compl_clear_autocomplete_delay(void);
DLLEXPORT bool ins_compl_autocomplete_pending(void);
DLLEXPORT int64_t ins_compl_autocomplete_elapsed(void);
DLLEXPORT void free_insexpand_stuff(void);
DLLEXPORT void f_preinserted(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
