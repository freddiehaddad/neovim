#pragma once

#include <stdbool.h>

#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/ex_eval_defs.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT bool aborting(void);
DLLEXPORT void update_force_abort(void);
DLLEXPORT bool should_abort(int retcode);
DLLEXPORT bool aborted_in_try(void) FUNC_ATTR_PURE;
DLLEXPORT bool cause_errthrow(const char *mesg, bool multiline, bool concat, bool severe,
                              bool *ignore) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void free_global_msglist(void);
DLLEXPORT void do_errthrow(cstack_T *cstack, char *cmdname);
DLLEXPORT bool do_intthrow(cstack_T *cstack);
DLLEXPORT char *get_exception_string(void *value, except_type_T type, char *cmdname,
                                     bool *should_free);
DLLEXPORT void discard_current_exception(void);
DLLEXPORT void exception_state_save(exception_state_T *estate);
DLLEXPORT void exception_state_restore(exception_state_T *estate);
DLLEXPORT void exception_state_clear(void);
DLLEXPORT void report_make_pending(int pending, void *value);
DLLEXPORT void ex_eval(exarg_T *eap);
DLLEXPORT void ex_if(exarg_T *eap);
DLLEXPORT void ex_endif(exarg_T *eap);
DLLEXPORT void ex_else(exarg_T *eap);
DLLEXPORT void ex_while(exarg_T *eap);
DLLEXPORT void ex_continue(exarg_T *eap);
DLLEXPORT void ex_break(exarg_T *eap);
DLLEXPORT void ex_endwhile(exarg_T *eap);
DLLEXPORT void ex_throw(exarg_T *eap);
DLLEXPORT void do_throw(cstack_T *cstack);
DLLEXPORT void ex_try(exarg_T *eap);
DLLEXPORT void ex_catch(exarg_T *eap);
DLLEXPORT void ex_finally(exarg_T *eap);
DLLEXPORT void ex_endtry(exarg_T *eap);
DLLEXPORT void enter_cleanup(cleanup_T *csp);
DLLEXPORT void leave_cleanup(cleanup_T *csp);
DLLEXPORT int cleanup_conditionals(cstack_T *cstack, int searched_cond, int inclusive);
DLLEXPORT void rewind_conditionals(cstack_T *cstack, int idx, int cond_type, int *cond_level);
DLLEXPORT void ex_endfunction(exarg_T *eap);
DLLEXPORT bool has_loop_cmd(char *p);
