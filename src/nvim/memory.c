// Various routines dealing with allocation and deallocation of memory.

#include <stdbool.h>
#include <stddef.h>

#include "nvim/api/extmark.h"
#include "nvim/api/private/helpers.h"
#include "nvim/api/ui.h"
#include "nvim/arglist.h"
#include "nvim/ascii_defs.h"
#include "nvim/buffer_defs.h"
#include "nvim/buffer_updates.h"
#include "nvim/channel.h"
#include "nvim/decoration_provider.h"
#include "nvim/drawline.h"
#include "nvim/errors.h"
#include "nvim/eval.h"
#include "nvim/gettext_defs.h"
#include "nvim/globals.h"
#include "nvim/highlight.h"
#include "nvim/highlight_group.h"
#include "nvim/input_cmdatom.h"
#include "nvim/insexpand.h"
#include "nvim/lua/executor.h"
#include "nvim/main.h"
#include "nvim/map_defs.h"
#include "nvim/mapping.h"
#include "nvim/mcursor.h"
#include "nvim/memfile.h"
#include "nvim/memory.h"
#include "nvim/message.h"
#include "nvim/normal.h"
#include "nvim/option_vars.h"
#include "nvim/sign.h"
#include "nvim/state_defs.h"
#include "nvim/statusline.h"
#include "nvim/types_defs.h"
#include "nvim/ui.h"
#include "nvim/ui_client.h"
#include "nvim/ui_compositor.h"
#include "nvim/usercmd.h"

#include "memory.c.generated.h"

#ifdef EXITFREE

# include "nvim/autocmd.h"
# include "nvim/buffer.h"
# include "nvim/cmdhist.h"
# include "nvim/diff.h"
# include "nvim/ex_cmds.h"
# include "nvim/ex_docmd.h"
# include "nvim/file_search.h"
# include "nvim/grid.h"
# include "nvim/input.h"
# include "nvim/insert.h"
# include "nvim/mark.h"
# include "nvim/msgpack_rpc/channel.h"
# include "nvim/option.h"
# include "nvim/os/os.h"
# include "nvim/quickfix.h"
# include "nvim/regexp.h"
# include "nvim/register.h"
# include "nvim/search.h"
# include "nvim/spell.h"
# include "nvim/tag.h"
# include "nvim/window.h"

bool entered_free_all_mem = false;

// Free everything that we allocated.
// Can be used to detect memory leaks, e.g., with ccmalloc.
// NOTE: This is tricky!  Things are freed that functions depend on.  Don't be
// surprised if Vim crashes...
// Some things can't be freed, esp. things local to a library function.
void free_all_mem(void)
{
  buf_T *buf, *nextbuf;

  // If a routine below recurses into free_all_mem, don't try freeing everything again.
  if (entered_free_all_mem) {
    // Except the Lua state. #39675
    nlua_free_all_mem();
    return;
  }
  entered_free_all_mem = true;
  // Don't want to trigger autocommands from here on.
  block_autocmds();

  // Close all tabs and windows.  Reset 'equalalways' to avoid redraws.
  p_ea = false;
  if (first_tabpage != NULL && first_tabpage->tp_next != NULL) {
    do_cmdline_cmd("tabonly!");
  }

  // Free all spell info.
  spell_free_all();

  // Clear user commands (before deleting buffers).
  ex_comclear(NULL);

  if (curbuf != NULL) {
    // Clear menus.
    do_cmdline_cmd("aunmenu *");
    do_cmdline_cmd("tlunmenu *");
    do_cmdline_cmd("menutranslate clear");

    // Clear mappings, abbreviations, breakpoints.
    // NB: curbuf not used with local=false arg
    map_clear_mode(curbuf, MAP_ALL_MODES, false, false);
    map_clear_mode(curbuf, MAP_ALL_MODES, false, true);
    do_cmdline_cmd("breakdel *");
    do_cmdline_cmd("profdel *");
    do_cmdline_cmd("set keymap=");
  }

  free_titles();
  free_findfile();

  // Obviously named calls.
  free_all_autocmds();
  free_all_marks();
  alist_clear(&global_alist);
  free_homedir();
  free_users();
  free_search_patterns();
  free_old_sub();
  free_last_insert();
  free_insexpand_stuff();
  free_prev_shellcmd();
  free_regexp_stuff();
  free_tag_stuff();
  free_cd_dir();
  free_signs();
  set_expr_line(NULL);
  if (curtab != NULL) {
    diff_clear(curtab);
  }
  clear_sb_text(true);            // free any scrollback text

  // Free some global vars.
  xfree(last_cmdline);
  xfree(new_last_cmdline);
  set_keep_msg(NULL, 0);

  // Clear cmdline history.
  p_hi = 0;
  init_history();

  free_quickfix();

  // Close all script inputs.
  close_all_scripts();

  if (curwin != NULL) {
    // Destroy all windows.  Must come before freeing buffers.
    win_free_all();
  }

  // Free all option values.  Must come after closing windows.
  free_all_options();

  // Free all buffers.  Reset 'autochdir' to avoid accessing things that
  // were freed already.
  // Must be after eval_clear to avoid it trying to access b:changedtick after
  // freeing it.
  p_acd = false;
  for (buf = firstbuf; buf != NULL;) {
    bufref_T bufref;
    set_bufref(&bufref, buf);
    nextbuf = buf->b_next;
    // All windows were freed.  Reset b_nwindows so buffers can be wiped.
    buf->b_nwindows = 0;

    // Since options (in addition to other stuff) have been freed above we need to ensure no
    // callbacks are called, so free them before closing the buffer.
    buf_free_callbacks(buf);

    close_buffer(NULL, buf, DOBUF_WIPE, false, false, false);
    // Didn't work, try next one.
    buf = bufref_valid(&bufref) ? nextbuf : firstbuf;
  }

  // Clear registers.
  clear_registers();
  redo_free_all();

  // highlight info
  free_highlight();

  reset_last_sourcing();

  if (first_tabpage != NULL) {
    free_tabpage(first_tabpage);
    first_tabpage = NULL;
  }

  // message history
  msg_hist_clear(0);

  channel_free_all_mem();
  eval_clear();
  api_extmark_free_all_mem();
  atom_free_all();
  mc_free_all();

  map_destroy(int, &buffer_handles);
  map_destroy(int, &window_handles);
  map_destroy(int, &tabpage_handles);

  // free screenlines (can't display anything now!)
  grid_free_all_mem();
  stl_clear_click_defs(tab_page_click_defs, tab_page_click_defs_size);
  xfree(tab_page_click_defs);

  clear_hl_tables(false);

  check_quickfix_busy();

  decor_free_all_mem();
  drawline_free_all_mem();

  if (ui_client_channel_id) {
    ui_client_free_all_mem();
  }

  remote_ui_free_all_mem();
  ui_free_all_mem();
  ui_comp_free_all_mem();
  nlua_free_all_mem();
  rpc_free_all_mem();
  autocmd_free_all_mem();

  // should be last, in case earlier free functions deallocates arenas
  arena_free_reuse_blks();
}

#endif
