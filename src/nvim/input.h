#pragma once

#include <stddef.h>  // IWYU pragma: keep
#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/event/defs.h"  // IWYU pragma: keep (MultiQueue)
#include "nvim/input_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// Argument for flush_buffers().
typedef enum {
  FLUSH_MINIMAL,
  FLUSH_TYPEAHEAD,  ///< flush current typebuf contents
  FLUSH_INPUT,      ///< flush typebuf and inchar() input
} flush_buffers_T;

enum { NSCRIPT = 15, };  ///< Maximum number of streams to read script from

EXTERN bool test_disable_char_avail INIT( = false);

#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT char *get_recorded(void);
DLLEXPORT String redo_keys(const CmdSpec *spec) FUNC_ATTR_WARN_UNUSED_RESULT;
DLLEXPORT CmdSpec redo_spec(void);
DLLEXPORT void sb_add_char(StringBuilder *sb, int c) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT bool stuff_empty(void) FUNC_ATTR_PURE;
DLLEXPORT bool readbuf1_empty(void) FUNC_ATTR_PURE;
DLLEXPORT void typeahead_noflush(int c);
DLLEXPORT void flush_buffers(flush_buffers_T flush_typeahead);
DLLEXPORT void block_on_key(void);
DLLEXPORT void unblock_on_key(void);
DLLEXPORT void beep_flush(void);
DLLEXPORT void redo_new(CmdSpec spec);
DLLEXPORT void redo_free_all(void);
DLLEXPORT void prep_redo(bool as_atom, bool arg_meta, CmdSpec spec);
DLLEXPORT void prep_redo_visual(const char *keys, size_t len, CmdSpec spec);
DLLEXPORT void redo_cancel(void);
DLLEXPORT void save_redobuff(RedoState *save_redo) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void restore_redobuff(RedoState *save_redo) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void redo_append_str(const char *s, ptrdiff_t len);
DLLEXPORT void sb_add_lit(StringBuilder *buf, const char *str, int len);
DLLEXPORT void redo_append_lit(const char *str, int len);
DLLEXPORT void sb_add_spec(StringBuilder *buf, const char *s);
DLLEXPORT void redo_append_spec(const char *s);
DLLEXPORT void redo_append_char(int c);
DLLEXPORT void stuffReadbuff(const char *s) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void stuffRedoReadbuff(const char *s) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void stuffReadbuffLen(const char *s, ptrdiff_t len);
DLLEXPORT void stuffReadbuffSpecial(const char *s) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void stuffcharReadbuff(int c);
DLLEXPORT void stuffnumReadbuff(int n);
DLLEXPORT void stuffescaped(const char *arg, bool literally) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int start_redo(int count, bool old_redo);
DLLEXPORT int start_redo_ins(void);
DLLEXPORT void stop_redo_ins(void);
DLLEXPORT bool noremap_keys(void) FUNC_ATTR_PURE;
DLLEXPORT int ins_typebuf(char *str, int noremap, int offset, bool nottyped, bool silent)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void requeue_key(int c, int modifiers, int extra_consumed, bool recorded);
DLLEXPORT bool typebuf_changed(int tb_change_cnt) FUNC_ATTR_PURE;
DLLEXPORT int typebuf_typed(void) FUNC_ATTR_PURE;
DLLEXPORT int typebuf_maplen(void) FUNC_ATTR_PURE;
DLLEXPORT void del_typebuf(int len, int offset);
DLLEXPORT void gotchars_ignore(void);
DLLEXPORT void may_sync_undo(void);
DLLEXPORT void save_typeahead(tasave_T *tp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void restore_typeahead(tasave_T *tp) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void openscript(char *name, bool directly);
DLLEXPORT void close_all_scripts(void);
DLLEXPORT bool open_scriptin(char *scriptin_name) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int using_script(void) FUNC_ATTR_PURE;
DLLEXPORT void before_blocking(void);
DLLEXPORT int merge_modifiers(int c_arg, int *modifiers) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int vgetc(void);
DLLEXPORT int safe_vgetc(void);
DLLEXPORT int plain_vgetc(void);
DLLEXPORT int vpeekc(void);
DLLEXPORT int vpeekc_any(void);
DLLEXPORT bool char_avail(void);
DLLEXPORT void f_getchar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcharstr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void f_getcharmod(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
DLLEXPORT void vungetc(int c);
DLLEXPORT void check_end_reg_executing(bool advance);
DLLEXPORT int fix_input_buffer(uint8_t *buf, int len) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT int get_keystroke(MultiQueue *events);
DLLEXPORT char *getcmdkeycmd(int promptc, void *cookie, int indent, bool do_concat);
DLLEXPORT bool map_execute_lua(bool may_repeat, bool discard);
DLLEXPORT void paste_store(const uint64_t channel_id, const TriState state, const String str,
                           const bool crlf);
DLLEXPORT void paste_repeat(int count);
