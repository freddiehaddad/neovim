#pragma once

#include <stdint.h>  // IWYU pragma: keep
#include <time.h>

#include "nvim/cmdexpand_defs.h"  // IWYU pragma: keep
#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/runtime_defs.h"  // IWYU pragma: keep

#define TIME_MSG(s) do { \
  if (time_fd != NULL) time_msg(s, NULL); \
} while (0)

proftime_T profile_start(void);
proftime_T profile_end(proftime_T tm);
const char *profile_msg(proftime_T tm);
proftime_T profile_setlimit(int64_t msec);
bool profile_passed_limit(proftime_T tm);
proftime_T profile_zero(void);
proftime_T profile_divide(proftime_T tm, int count);
proftime_T profile_add(proftime_T tm1, proftime_T tm2);
proftime_T profile_sub(proftime_T tm1, proftime_T tm2);
proftime_T profile_self(proftime_T self, proftime_T total, proftime_T children);
void profile_set_wait(proftime_T wait);
proftime_T profile_sub_wait(proftime_T tm, proftime_T tma);
int64_t profile_signed(proftime_T tm);
int profile_cmp(proftime_T tm1, proftime_T tm2);
void profile_reset(void);
void ex_profile(exarg_T *eap);
char *get_profile_name(expand_T *xp, int idx);
void set_context_in_profile_cmd(expand_T *xp, const char *arg);
void prof_input_start(void);
void prof_input_end(void);
bool prof_def_func(void);
void func_do_profile(ufunc_T *fp);
void prof_child_enter(proftime_T *tm);
void prof_child_exit(proftime_T *tm);
void func_line_start(void *cookie);
void func_line_exec(void *cookie);
void func_line_end(void *cookie);
void profile_init(scriptitem_T *si);
void script_prof_save(proftime_T *tm);
void script_prof_restore(const proftime_T *tm);
void profile_dump(void);
void script_line_start(void);
void script_line_exec(void);
void script_line_end(void);
void time_push(proftime_T *rel, proftime_T *start);
void time_pop(proftime_T tp);
void time_start(const char *message);
void time_msg(const char *mesg, const proftime_T *start);
void time_init(const char *fname, const char *proc_name);
void time_finish(void);
