#pragma once

#include <stdbool.h>

#include "nvim/register_defs.h"

yankreg_T *adjust_clipboard_name(int *name, bool quiet, bool writing);
bool get_clipboard(int name, yankreg_T **target, bool quiet);
void set_clipboard(int name, yankreg_T *reg);
void start_batch_changes(void);
void end_batch_changes(void);
