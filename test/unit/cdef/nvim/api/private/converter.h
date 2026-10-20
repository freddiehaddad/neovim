#pragma once

#include <stdbool.h>

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/memory_defs.h"  // IWYU pragma: keep

Object vim_to_object(typval_T *obj, Arena *arena, bool reuse_strdata);
void object_to_vim(Object obj, typval_T *tv, Error *err);
void object_to_vim_take_luaref(Object *obj, typval_T *tv, bool take_luaref, Error *err);
