#pragma once

#include <stdbool.h>  // IWYU pragma: keep

extern char *confirm_msg;      ///< ":confirm" message
extern char *confirm_buttons;  ///< ":confirm" buttons sent to cmdline as prompt

int ask_yesno(const char *const str);
int prompt_for_input(char *prompt, int hl_id, bool one_key, bool *mouse_used);
int do_dialog(int type, const char *title, const char *message, const char *buttons, int dfltbutton,
              const char *textfield, int ex_cmd);
int vim_dialog_yesno(int type, char *title, char *message, int dflt);
int vim_dialog_yesnocancel(int type, char *title, char *message, int dflt);
int vim_dialog_yesnoallcancel(int type, char *title, char *message, int dflt);
