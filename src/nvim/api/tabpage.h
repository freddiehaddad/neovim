#pragma once

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep

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
DLLEXPORT ArrayOf(Window) nvim_tabpage_list_wins(Tabpage tabpage, Arena *arena,
                                                 Error *err) FUNC_API_SINCE(1);
DLLEXPORT Object nvim_tabpage_get_var(Tabpage tabpage, String name, Arena *arena,
                                      Error *err) FUNC_API_SINCE(1);
DLLEXPORT void nvim_tabpage_set_var(Tabpage tabpage, String name, Object value,
                                    Error *err) FUNC_API_SINCE(1);
DLLEXPORT void nvim_tabpage_del_var(Tabpage tabpage, String name, Error *err) FUNC_API_SINCE(1);
DLLEXPORT Window nvim_tabpage_get_win(Tabpage tabpage, Error *err) FUNC_API_SINCE(1);
DLLEXPORT void nvim_tabpage_set_win(Tabpage tabpage, Window win, Error *err) FUNC_API_SINCE(12);
DLLEXPORT Integer nvim_tabpage_get_number(Tabpage tabpage, Error *err) FUNC_API_SINCE(1);
DLLEXPORT Boolean nvim_tabpage_is_valid(Tabpage tabpage) FUNC_API_SINCE(1);
DLLEXPORT Tabpage nvim_open_tabpage(Buffer buf, Boolean enter, Dict(tabpage_config) *config,
                                    Error *err) FUNC_API_SINCE(14) FUNC_API_TEXTLOCK;
#include "nvim/func_attr.h"
