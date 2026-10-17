#pragma once

#include <lua.h>  // IWYU pragma: keep
#include <stdbool.h>

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/memory_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

#define nlua_pop_Buffer nlua_pop_handle
#define nlua_pop_Window nlua_pop_handle
#define nlua_pop_Tabpage nlua_pop_handle

#define nlua_push_Buffer nlua_push_handle
#define nlua_push_Window nlua_push_handle
#define nlua_push_Tabpage nlua_push_handle

/// Flags for nlua_push_*() functions.
enum {
  kNluaPushSpecial = 0x01,   ///< Use lua-special-tbl when necessary
  kNluaPushFreeRefs = 0x02,  ///< Free luarefs to elide an api_luarefs_free_*() later
};

bool nlua_pop_typval(lua_State *lstate, typval_T *ret_tv);
bool nlua_push_typval(lua_State *lstate, typval_T *const tv, int flags);
void nlua_push_String(lua_State *lstate, const String s, int flags);
void nlua_push_Integer(lua_State *lstate, const Integer n, int flags);
void nlua_push_Float(lua_State *lstate, const Float f, int flags);
void nlua_push_Boolean(lua_State *lstate, const Boolean b, int flags);
void nlua_push_Dict(lua_State *lstate, const Dict dict, int flags);
void nlua_push_Array(lua_State *lstate, const Array array, int flags);
void nlua_push_handle(lua_State *lstate, const handle_T item, int flags);
void nlua_push_Object(lua_State *lstate, Object *obj, int flags);
String nlua_pop_String(lua_State *lstate, Arena *arena, Error *err);
Integer nlua_pop_Integer(lua_State *lstate, Arena *arena, Error *err);
Boolean nlua_pop_Boolean(lua_State *lstate, Arena *arena, Error *err);
Boolean nlua_pop_Boolean_strict(lua_State *lstate, Error *err);
Float nlua_pop_Float(lua_State *lstate, Arena *arena, Error *err);
Array nlua_pop_Array(lua_State *lstate, Arena *arena, Error *err);
Dict nlua_pop_Dict(lua_State *lstate, bool ref, Arena *arena, Error *err);
Object nlua_pop_Object(lua_State *const lstate, bool ref, Arena *arena, Error *const err);
LuaRef nlua_pop_LuaRef(lua_State *const lstate, Arena *arena, Error *err);
handle_T nlua_pop_handle(lua_State *lstate, Arena *arena, Error *err);
void nlua_init_types(lua_State *const lstate);
void nlua_pop_keydict(lua_State *L, void *retval, FieldHashfn hashy, char **err_opt, Arena *arena,
                      Error *err);
void nlua_push_keydict(lua_State *L, void *value, KeySetLink *table);
