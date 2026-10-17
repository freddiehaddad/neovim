#pragma once

#include "klib/kvec.h"
#include "nvim/os/input.h"

#ifndef HPCON
# define HPCON VOID *
#endif

extern HRESULT(WINAPI *pCreatePseudoConsole)
  (COORD, HANDLE, HANDLE, DWORD, HPCON *);
extern HRESULT(WINAPI *pResizePseudoConsole)(HPCON, COORD);
extern void(WINAPI *pClosePseudoConsole)(HPCON);

typedef struct conpty {
  HPCON pty;
  STARTUPINFOEXW si_ex;
} conpty_t;

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
DLLEXPORT bool os_has_conpty_working(void);
DLLEXPORT TriState os_dyn_conpty_init(void);
DLLEXPORT conpty_t *os_conpty_init(char **in_name, char **out_name, uint16_t width,
                                   uint16_t height);
DLLEXPORT bool os_conpty_spawn(conpty_t *conpty_object, HANDLE *proc_handle, wchar_t *name,
                               wchar_t *cmd_line, wchar_t *cwd, wchar_t *env);
DLLEXPORT void os_conpty_set_size(conpty_t *conpty_object, uint16_t width, uint16_t height);
DLLEXPORT void os_conpty_free(void *data);
#include "nvim/func_attr.h"
