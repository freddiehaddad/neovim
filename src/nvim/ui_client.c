/// Nvim's own UI client, which attaches to a child or remote Nvim server.

#include <stdbool.h>
#include <stddef.h>
#include <string.h>

#include "nvim/api/private/defs.h"
#include "nvim/highlight_defs.h"
#include "nvim/log.h"
#include "nvim/tui/tui.h"
#include "nvim/tui/tui_defs.h"
#include "nvim/ui_client.h"
#include "nvim/ui_defs.h"

extern TUIData *tui;
HlAttrs ui_client_dict2hlattrs(Dict d, bool rgb);

// uncrustify:off
#include "ui_client.c.generated.h"
#include "ui_events_client.generated.h"
// uncrustify:on

UIClientHandler ui_client_get_redraw_handler_at(int hash)
{
  return event_handlers[hash];
}
