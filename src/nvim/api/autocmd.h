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
DLLEXPORT ArrayOf(DictAs(get_autocmds__ret)) nvim_get_autocmds(Dict(get_autocmds) *opts,
                                                               Arena *arena,
                                                               Error *err) FUNC_API_SINCE(9);
DLLEXPORT Integer nvim_create_autocmd(uint64_t channel_id, Object event,
                                      Dict(create_autocmd) *opts, Arena *arena,
                                      Error *err) FUNC_API_SINCE(9) FUNC_API_FAST;
DLLEXPORT void nvim_del_autocmd(Integer id, Error *err) FUNC_API_SINCE(9);
DLLEXPORT void nvim_clear_autocmds(Dict(clear_autocmds) *opts, Arena *arena,
                                   Error *err) FUNC_API_SINCE(9);
DLLEXPORT Integer nvim_create_augroup(uint64_t channel_id, String name,
                                      Dict(create_augroup) *opts, Error *err) FUNC_API_SINCE(9);
DLLEXPORT void nvim_del_augroup_by_id(Integer id, Error *err) FUNC_API_SINCE(9);
DLLEXPORT void nvim_del_augroup_by_name(String name, Error *err) FUNC_API_SINCE(9);
DLLEXPORT void nvim_exec_autocmds(Object event, Dict(exec_autocmds) *opts, Arena *arena,
                                  Error *err) FUNC_API_SINCE(9);
#include "nvim/func_attr.h"
