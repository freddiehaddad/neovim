#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "nvim/api/keysets_defs.h"
#include "nvim/api/private/defs.h"
#include "nvim/cmdexpand_defs.h"
#include "nvim/func_attr.h"
#include "nvim/highlight_defs.h"
#include "nvim/types_defs.h"

enum { MAX_HL_ID = 20000, };  ///< maximum value for a highlight ID.

typedef struct {
  char *name;
  RgbValue color;
} color_name_table_T;
extern color_name_table_T color_name_table[708];

int highlight_num_groups(void);
char *highlight_group_name(int id);
int highlight_link_id(int id);
void syn_init_cmdline_highlight(bool reset, bool init);
void init_highlight(bool both, bool reset);
int load_colors(char *name);
void set_hl_group(int id, HlAttrs attrs, Dict(highlight) *dict, int link_id);
void do_highlight(const char *line, const bool forceit, const bool init) FUNC_ATTR_NONNULL_ALL;
void free_highlight(void);
void restore_cterm_colors(void);
Dict ns_get_hl_defs(NS ns_id, Dict(get_highlight) *opts, Arena *arena, Error *err);
const char *highlight_has_attr(const int id, const int flag, const int modec)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_PURE;
const char *highlight_color(const int id, const char *const what, const int modec)
  FUNC_ATTR_WARN_UNUSED_RESULT FUNC_ATTR_NONNULL_ALL;
bool syn_list_header(const bool did_header, const int outlen, const int id, bool force_newline);
int syn_name2id(const char *name) FUNC_ATTR_NONNULL_ALL;
int syn_name2id_len(const char *name, size_t len) FUNC_ATTR_NONNULL_ALL;
int syn_name2attr(const char *name) FUNC_ATTR_NONNULL_ALL;
int highlight_exists(const char *name);
char *syn_id2name(int id);
int syn_check_group(const char *name, size_t len);
int syn_id2attr(int hl_id);
int syn_ns_id2attr(int ns_id, int hl_id, bool *optional) FUNC_ATTR_NONNULL_ALL;
int syn_get_final_id(int hl_id);
bool syn_ns_get_final_id(int *ns_id, int *hl_idp);
void highlight_attr_set_all(void);
void highlight_changed(void);
void set_context_in_highlight_cmd(expand_T *xp, const char *arg);
char *get_highlight_name(expand_T *const xp, int idx) FUNC_ATTR_WARN_UNUSED_RESULT;
const char *get_highlight_name_ext(expand_T *xp, int idx, bool skip_cleared)
  FUNC_ATTR_WARN_UNUSED_RESULT;
RgbValue name_to_color(const char *name, int *idx);
const char *coloridx_to_name(int idx, int val, char hexbuf[8]);
int name_to_ctermcolor(const char *name);
