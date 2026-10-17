#pragma once

#include <stdint.h>  // IWYU pragma: keep

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
DLLEXPORT Dict nvim_exec2(uint64_t channel_id, String src, Dict(exec_opts) *opts,
                          Error *err) FUNC_API_SINCE(11) FUNC_API_RET_ALLOC;
DLLEXPORT String exec_impl(uint64_t channel_id, String src, Dict(exec_opts) *opts, Error *err);
DLLEXPORT void nvim_command(String cmd, Error *err) FUNC_API_SINCE(1);
DLLEXPORT Object nvim_eval(String expr, Arena *arena, Error *err) FUNC_API_SINCE(1);
DLLEXPORT Object nvim_call_function(uint64_t channel_id, String fn, Array args, Arena *arena,
                                    Error *err) FUNC_API_SINCE(1);
DLLEXPORT Object nvim_call_dict_function(uint64_t channel_id, Object dict, String fn, Array args,
                                         Arena *arena, Error *err) FUNC_API_SINCE(4);
DLLEXPORT Dict nvim_parse_expression(String expr, String flags, Boolean hl, Arena *arena,
                                     Error *err) FUNC_API_SINCE(4) FUNC_API_FAST;
#include "nvim/func_attr.h"
