#pragma once

#include <lua.h>  // IWYU pragma: keep

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT int nlua_spell_check(lua_State *lstate) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int luaopen_spell(lua_State *L) FUNC_ATTR_NONNULL_ALL;
