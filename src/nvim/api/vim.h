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

DLLEXPORT Integer nvim_get_hl_id_by_name(String name, Error *err) FUNC_API_SINCE(7);
DLLEXPORT DictAs(get_hl_info) nvim_get_hl(Integer ns_id, Dict(get_highlight) *opts, Arena *arena,
                                          Error *err) FUNC_API_SINCE(11);
DLLEXPORT void nvim_set_hl(uint64_t channel_id, Integer ns_id, String name, Dict(highlight) *val,
                           Error *err) FUNC_API_SINCE(7);
DLLEXPORT Integer nvim_get_hl_ns(Dict(get_ns) *opts, Error *err) FUNC_API_SINCE(12);
DLLEXPORT void nvim_set_hl_ns(Integer ns_id, Error *err) FUNC_API_SINCE(10);
DLLEXPORT void nvim_set_hl_ns_fast(Integer ns_id, Error *err) FUNC_API_SINCE(10) FUNC_API_FAST;
DLLEXPORT void nvim_feedkeys(String keys, String mode, Boolean escape_ks) FUNC_API_SINCE(1);
DLLEXPORT Integer nvim_input(uint64_t channel_id, String keys) FUNC_API_SINCE(1) FUNC_API_FAST;
DLLEXPORT void nvim_input_mouse(String button, String action, String modifier, Integer grid,
                                Integer row, Integer col, Error *err) FUNC_API_SINCE(6)
  FUNC_API_FAST;
DLLEXPORT String nvim_replace_termcodes(String str, Boolean from_part, Boolean do_lt,
                                        Boolean special) FUNC_API_SINCE(1) FUNC_API_FAST
  FUNC_API_RET_ALLOC;
DLLEXPORT Object nvim_exec_lua(String code, Array args, Arena *arena,
                               Error *err) FUNC_API_SINCE(7) FUNC_API_REMOTE_ONLY;
DLLEXPORT Object nvim__exec_lua_fast(String code, Array args, Arena *arena,
                                     Error *err) FUNC_API_SINCE(14) FUNC_API_REMOTE_ONLY
  FUNC_API_FAST;
DLLEXPORT Integer nvim_strwidth(String text, Error *err) FUNC_API_SINCE(1);
DLLEXPORT ArrayOf(String) nvim_list_runtime_paths(Arena *arena, Error *err) FUNC_API_SINCE(1);
DLLEXPORT Array nvim__runtime_inspect(Arena *arena);
DLLEXPORT ArrayOf(String) nvim_get_runtime_file(String name, Boolean all, Arena *arena,
                                                Error *err) FUNC_API_SINCE(7) FUNC_API_FAST;
DLLEXPORT String nvim__get_lib_dir(void) FUNC_API_RET_ALLOC;
DLLEXPORT ArrayOf(String) nvim__get_runtime(ArrayOf(String) pat, Boolean all, Dict(runtime) *opts,
                                            Arena *arena, Error *err) FUNC_API_SINCE(8)
  FUNC_API_FAST;
DLLEXPORT void nvim_set_current_dir(String dir, Error *err) FUNC_API_SINCE(1);
DLLEXPORT String nvim_get_current_line(Arena *arena, Error *err) FUNC_API_SINCE(1);
DLLEXPORT void nvim_set_current_line(String line, Arena *arena,
                                     Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;
DLLEXPORT void nvim_del_current_line(Arena *arena, Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;
DLLEXPORT Object nvim_get_var(String name, Arena *arena, Error *err) FUNC_API_SINCE(1);
DLLEXPORT void nvim_set_var(String name, Object value, Error *err) FUNC_API_SINCE(1);
DLLEXPORT void nvim_del_var(String name, Error *err) FUNC_API_SINCE(1);
DLLEXPORT Object nvim_get_vvar(String name, Arena *arena, Error *err) FUNC_API_SINCE(1);
DLLEXPORT void nvim_set_vvar(String name, Object value, Error *err) FUNC_API_SINCE(6);
DLLEXPORT Union(Integer, String) nvim_echo(ArrayOf(Tuple(String, *HLGroupID)) chunks,
                                           Boolean history, Dict(echo_opts) *opts,
                                           Error *err) FUNC_API_SINCE(7);
DLLEXPORT ArrayOf(Buffer) nvim_list_bufs(Arena *arena) FUNC_API_SINCE(1);
DLLEXPORT Buffer nvim_get_current_buf(void) FUNC_API_SINCE(1);
DLLEXPORT void nvim_set_current_buf(Buffer buf, Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;
DLLEXPORT ArrayOf(Window) nvim_list_wins(Arena *arena) FUNC_API_SINCE(1);
DLLEXPORT Window nvim_get_current_win(void) FUNC_API_SINCE(1);
DLLEXPORT void nvim_set_current_win(Window win, Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;
DLLEXPORT Buffer nvim_create_buf(Boolean listed, Boolean scratch, Error *err) FUNC_API_SINCE(6);
DLLEXPORT Integer nvim_open_term(Buffer buf, Dict(open_term) *opts,
                                 Error *err) FUNC_API_SINCE(7) FUNC_API_TEXTLOCK;
DLLEXPORT void nvim_chan_send(Integer chan, String data,
                              Error *err) FUNC_API_SINCE(7) FUNC_API_REMOTE_ONLY FUNC_API_LUA_ONLY;
DLLEXPORT ArrayOf(Tabpage) nvim_list_tabpages(Arena *arena) FUNC_API_SINCE(1);
DLLEXPORT Tabpage nvim_get_current_tabpage(void) FUNC_API_SINCE(1);
DLLEXPORT void nvim_set_current_tabpage(Tabpage tabpage,
                                        Error *err) FUNC_API_SINCE(1) FUNC_API_TEXTLOCK;
DLLEXPORT Boolean nvim_paste(uint64_t channel_id, String data, Boolean crlf, Integer phase,
                             Arena *arena, Error *err) FUNC_API_SINCE(6) FUNC_API_TEXTLOCK;
DLLEXPORT void nvim_put(ArrayOf(String) lines, String type, Boolean after, Boolean follow,
                        Arena *arena, Error *err) FUNC_API_SINCE(6) FUNC_API_TEXTLOCK;
DLLEXPORT Integer nvim_get_color_by_name(String name) FUNC_API_SINCE(1);
DLLEXPORT DictOf(Integer) nvim_get_color_map(Arena *arena) FUNC_API_SINCE(1);
DLLEXPORT Dict nvim_get_context(Dict(context) *opts, Arena *arena, Error *err) FUNC_API_SINCE(6);
DLLEXPORT Object nvim_load_context(Dict dict, Error *err) FUNC_API_SINCE(6);
DLLEXPORT DictAs(get_mode) nvim_get_mode(Arena *arena) FUNC_API_SINCE(2) FUNC_API_FAST;
DLLEXPORT ArrayOf(DictAs(get_keymap)) nvim_get_keymap(String mode, Arena *arena) FUNC_API_SINCE(3);
DLLEXPORT void nvim_set_keymap(uint64_t channel_id, String mode, String lhs, String rhs,
                               Dict(keymap) *opts, Error *err) FUNC_API_SINCE(6);
DLLEXPORT void nvim_del_keymap(uint64_t channel_id, String mode, String lhs, Dict(keymap_del) *opts,
                               Error *err) FUNC_API_SINCE(6);
DLLEXPORT ArrayOf(Object, 2) nvim_get_api_info(uint64_t channel_id,
                                               Arena *arena) FUNC_API_SINCE(1) FUNC_API_FAST
  FUNC_API_REMOTE_ONLY;
DLLEXPORT void nvim_set_client_info(uint64_t channel_id, String name, Dict version, String type,
                                    Dict methods, Dict attributes, Arena *arena,
                                    Error *err) FUNC_API_SINCE(4) FUNC_API_REMOTE_ONLY;
DLLEXPORT void nvim_chan_set(uint64_t channel_id, Integer chan, Dict(chan_set) *config,
                             Error *err) FUNC_API_SINCE(15) FUNC_API_FAST;
DLLEXPORT void nvim__cmdwin_set(String type, Buffer buf, Error *err) FUNC_API_SINCE(14);
DLLEXPORT Dict nvim_get_chan_info(uint64_t channel_id, Integer chan, Arena *arena,
                                  Error *err) FUNC_API_SINCE(4);
DLLEXPORT ArrayOf(Dict) nvim_list_chans(Arena *arena) FUNC_API_SINCE(4);
DLLEXPORT Object nvim__id(Object obj, Arena *arena);
DLLEXPORT Array nvim__id_array(Array arr, Arena *arena);
DLLEXPORT Dict nvim__id_dict(Dict dct, Arena *arena);
DLLEXPORT Float nvim__id_float(Float flt);
DLLEXPORT Dict nvim__stats(Arena *arena);
DLLEXPORT ArrayOf(Dict) nvim_list_uis(Arena *arena) FUNC_API_SINCE(4);
DLLEXPORT Array nvim_get_proc_children(Integer pid, Arena *arena, Error *err) FUNC_API_SINCE(4);
DLLEXPORT Object nvim_get_proc(Integer pid, Arena *arena, Error *err) FUNC_API_SINCE(4);
DLLEXPORT void nvim_select_popupmenu_item(Integer item, Boolean insert, Boolean finish,
                                          Dict(empty) *opts, Error *err) FUNC_API_SINCE(6);
DLLEXPORT Array nvim__inspect_cell(Integer grid, Integer row, Integer col, Arena *arena,
                                   Error *err);
DLLEXPORT void nvim__screenshot(String path) FUNC_API_FAST;
DLLEXPORT void nvim__invalidate_glyph_cache(void);
DLLEXPORT Object nvim__unpack(String str, Arena *arena, Error *err) FUNC_API_FAST;
DLLEXPORT Boolean nvim_del_mark(String name, Error *err) FUNC_API_SINCE(8);
DLLEXPORT Tuple(Integer, Integer, Buffer, String) nvim_get_mark(String name, Dict(empty) *opts,
                                                                Arena *arena,
                                                                Error *err) FUNC_API_SINCE(8);
DLLEXPORT DictAs(eval_statusline_ret) nvim_eval_statusline(String str, Dict(eval_statusline) *opts,
                                                           Arena *arena,
                                                           Error *err) FUNC_API_SINCE(8)
  FUNC_API_FAST;
DLLEXPORT DictOf(Integer) nvim__complete_set(Integer index, Dict(complete_set) *opts, Arena *arena,
                                             Error *err);
DLLEXPORT void nvim__redraw(Dict(redraw) *opts, Error *err) FUNC_API_SINCE(12);
DLLEXPORT void nvim__set_restart_on_crash(String progpath, Array argv);

#include "nvim/func_attr.h"
