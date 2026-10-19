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

UIClientHandler ui_client_get_redraw_handler(const char *name, size_t name_len, Error *error)
{
  int hash = ui_client_handler_hash(name, name_len);
  if (hash < 0) {
    return (UIClientHandler){ NULL, NULL };
  }
  return event_handlers[hash];
}
