#pragma once

#include <stddef.h>

#include "nvim/vterm/vterm_defs.h"
#include "nvim/vterm/vterm_internal_defs.h"

VTermEncoding *vterm_lookup_encoding(VTermEncodingType type, char designation);
