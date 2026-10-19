#include "nvim/api/private/defs.h"
#include "nvim/api/private/helpers.h"
#include "nvim/api/ui.h"
#include "nvim/ui.h"
#include "nvim/ui_defs.h"

void push_call(RemoteUI *ui, const char *name, Array args);

#include "api/ui.c.generated.h"
#include "ui_events_remote.generated.h"  // IWYU pragma: export
