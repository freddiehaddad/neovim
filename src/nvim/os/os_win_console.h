#pragma once

#include <stdbool.h>

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
DLLEXPORT void os_enable_ctrl_c(void);
DLLEXPORT int os_open_conin_fd(void);
DLLEXPORT void os_clear_hwnd(void);
DLLEXPORT void os_redirect_stdin_to_conin(void);
DLLEXPORT void os_redirect_stdout_stderr_to_conout(void);
DLLEXPORT void os_reattach_console_stdio(void);
DLLEXPORT void os_swap_to_hidden_console(void);
DLLEXPORT void os_attach_parent_console(void);
DLLEXPORT void os_detach_ui_console(bool stdio_ui);
DLLEXPORT void os_icon_reset(void);
DLLEXPORT void os_icon_init(void);
DLLEXPORT void os_title_save(void);
DLLEXPORT void os_title_reset(void);
DLLEXPORT void os_tty_guess_term(const char **term, int out_fd);
#include "nvim/func_attr.h"

#ifndef ENABLE_VIRTUAL_TERMINAL_INPUT
# define ENABLE_VIRTUAL_TERMINAL_INPUT 0x0200
#endif
