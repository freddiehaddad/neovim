#pragma once

#include <lua.h>  // IWYU pragma: keep
#include <stdint.h>
#include <tree_sitter/api.h>

#include "nvim/macros_defs.h"

TSTree *nts_parser_parse_buf(TSParser *p, const TSTree *old_tree, int bufnr, uint64_t timeout_ns);
void nlua_treesitter_free(void);
void nlua_treesitter_init(lua_State *const lstate) FUNC_ATTR_NONNULL_ALL;

EXTERN uint64_t tslua_query_parse_count INIT( = 0);
