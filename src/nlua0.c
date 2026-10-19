#include <lua.h>

int nvim_luaopen_nlua0(lua_State *L);
LUA_API int luaopen_nlua0(lua_State *L);

LUA_API int luaopen_nlua0(lua_State *L)
{
  return nvim_luaopen_nlua0(L);
}
