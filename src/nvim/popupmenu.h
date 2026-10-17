#pragma once

#include <stdbool.h>

#include "nvim/buffer_defs.h"
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/grid_defs.h"
#include "nvim/macros_defs.h"
#include "nvim/menu_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// Used for popup menu items.
typedef struct {
  char *pum_text;       ///< main menu text
  char *pum_kind;       ///< extra kind text (may be truncated)
  char *pum_extra;      ///< extra menu text (may be truncated)
  char *pum_info;       ///< extra info
  int pum_cpt_source_idx;    ///< index of completion source in 'cpt'
  int pum_user_abbr_hlattr;  ///< highlight attribute for abbr
  int pum_user_kind_hlattr;  ///< highlight attribute for kind
} pumitem_T;

EXTERN ScreenGrid pum_grid INIT( = SCREEN_GRID_INIT);

/// state for pum_ext_select_item.
EXTERN struct {
  bool active;
  int item;
  bool insert;
  bool finish;
} pum_want;

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void pum_display(pumitem_T *array, int size, int selected, bool array_changed,
                           int cmd_startcol);
DLLEXPORT void pum_redraw(void);
DLLEXPORT win_T *pum_set_info(int selected, char *info);
DLLEXPORT void pum_undisplay(bool immediate);
DLLEXPORT void pum_check_clear(void);
DLLEXPORT void pum_clear(void);
DLLEXPORT bool pum_visible(void);
DLLEXPORT bool pum_drawn(void);
DLLEXPORT void pum_invalidate(void);
DLLEXPORT void pum_recompose(void);
DLLEXPORT void pum_ext_select_item(int item, bool insert, bool finish);
DLLEXPORT int pum_get_height(void);
DLLEXPORT void pum_set_event_info(dict_T *dict);
DLLEXPORT void pum_show_popupmenu(vimmenu_T *menu);
DLLEXPORT void pum_make_popup(const char *path_name, int use_mouse_pos);
DLLEXPORT void pum_pos_ui_flush(bool force);
