#pragma once

#include <stdbool.h>
#include <stddef.h>  // IWYU pragma: keep

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/memory_defs.h"  // IWYU pragma: keep

bool os_proc_tree_kill(int pid, int sig);
int os_proc_children(int ppid, int **proc_list, size_t *proc_count);
#ifdef MSWIN
Dict os_proc_info(int pid, Arena *arena);
#endif
bool os_proc_running(int pid);
