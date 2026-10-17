#pragma once

#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"
#include "nvim/menu_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void ex_menu(exarg_T *eap);
DLLEXPORT bool menu_get(char *const path_name, int modes, list_T *list);
DLLEXPORT char *set_context_in_menu_cmd(expand_T *xp, const char *cmd, char *arg, bool forceit)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT char *get_menu_name(expand_T *xp, int idx);
DLLEXPORT char *get_menu_names(expand_T *xp, int idx);
DLLEXPORT int get_menu_cmd_modes(const char *cmd, bool forceit, int *noremap, bool *unmenu);
DLLEXPORT bool menu_is_menubar(const char *const name)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool menu_is_popup(const char *const name)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool menu_is_toolbar(const char *const name)
  FUNC_ATTR_PURE FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool menu_is_separator(char *name);
DLLEXPORT int get_menu_mode_flag(void);
DLLEXPORT void show_popupmenu(void);
DLLEXPORT void execute_menu(const exarg_T *eap, vimmenu_T *menu, int mode_idx)
  FUNC_ATTR_NONNULL_ARG(2);
DLLEXPORT void ex_emenu(exarg_T *eap);
DLLEXPORT vimmenu_T *menu_find(const char *path_name);
DLLEXPORT void ex_menutranslate(exarg_T *eap);
DLLEXPORT void f_menu_info(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
