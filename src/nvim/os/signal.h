#pragma once

#define DEFINE_FUNC_ATTRIBUTES
#include "nvim/func_attr.h"
#undef DEFINE_FUNC_ATTRIBUTES
#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void signal_init(void);
DLLEXPORT void signal_teardown(void);
DLLEXPORT void signal_start(void);
DLLEXPORT void signal_stop(void);
DLLEXPORT void signal_reject_deadly(void);
DLLEXPORT void signal_accept_deadly(void);
