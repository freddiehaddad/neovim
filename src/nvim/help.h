#pragma once

#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep

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

DLLEXPORT void ex_help(exarg_T *eap);
DLLEXPORT void ex_helpclose(exarg_T *eap);
DLLEXPORT char *check_help_lang(char *arg);
DLLEXPORT int help_heuristic(char *matched_string, int offset, bool wrong_case) FUNC_ATTR_PURE;
DLLEXPORT int find_help_tags(const char *arg, int *num_matches, char ***matches, bool keep_lang);
DLLEXPORT void cleanup_help_tags(int num_file, char **file);
DLLEXPORT void prepare_help_buffer(void);
DLLEXPORT void get_local_additions(void);
DLLEXPORT void ex_exusage(exarg_T *eap);
DLLEXPORT void ex_viusage(exarg_T *eap);
DLLEXPORT void ex_helptags(exarg_T *eap);

#include "nvim/func_attr.h"
