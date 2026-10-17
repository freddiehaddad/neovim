#pragma once

#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

/// flags for check_changed()
enum {
  CCGD_AW      = 1,   ///< do autowrite if buffer was changed
  CCGD_MULTWIN = 2,   ///< check also when several wins for the buf
  CCGD_FORCEIT = 4,   ///< ! used
  CCGD_ALLBUF  = 8,   ///< may write all buffers
  CCGD_EXCMD   = 16,  ///< may suggest using !
};

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

DLLEXPORT void ex_ruby(exarg_T *eap);
DLLEXPORT void ex_rubyfile(exarg_T *eap);
DLLEXPORT void ex_rubydo(exarg_T *eap);
DLLEXPORT void ex_python3(exarg_T *eap);
DLLEXPORT void ex_py3file(exarg_T *eap);
DLLEXPORT void ex_pydo3(exarg_T *eap);
DLLEXPORT void ex_perl(exarg_T *eap);
DLLEXPORT void ex_perlfile(exarg_T *eap);
DLLEXPORT void ex_perldo(exarg_T *eap);
DLLEXPORT int autowrite(buf_T *buf, bool forceit);
DLLEXPORT void autowrite_all(void);
DLLEXPORT bool check_changed(buf_T *buf, int flags);
DLLEXPORT void dialog_changed(buf_T *buf, bool checkall);
DLLEXPORT bool dialog_close_terminal(buf_T *buf);
DLLEXPORT bool can_abandon(buf_T *buf, bool forceit);
DLLEXPORT bool check_changed_any(bool hidden, bool unload);
DLLEXPORT int check_fname(void);
DLLEXPORT int buf_write_all(buf_T *buf, bool forceit);
DLLEXPORT void ex_listdo(exarg_T *eap);
DLLEXPORT void ex_compiler(exarg_T *eap);
DLLEXPORT void ex_checktime(exarg_T *eap);
DLLEXPORT void ex_drop(exarg_T *eap);

#include "nvim/func_attr.h"
