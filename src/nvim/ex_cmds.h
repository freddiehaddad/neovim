#pragma once

#include "nvim/ex_cmds_defs.h"  // IWYU pragma: keep

/// flags for do_ecmd()
enum {
  ECMD_HIDE       = 0x01,  ///< don't free the current buffer
  ECMD_SET_HELP   = 0x02,  ///< set b_help flag of (new) buffer before opening file
  ECMD_OLDBUF     = 0x04,  ///< use existing buffer if it exists
  ECMD_FORCEIT    = 0x08,  ///< ! used in Ex command
  ECMD_ADDBUF     = 0x10,  ///< don't edit, just add to buffer list
  ECMD_ALTBUF     = 0x20,  ///< like ECMD_ADDBUF and set the alternate file
  ECMD_NOWINENTER = 0x40,  ///< do not trigger BufWinEnter
};

/// for lnum argument in do_ecmd()
enum {
  ECMD_LASTL = 0,   ///< use last position in loaded file
  ECMD_LAST  = -1,  ///< use last position in all files
  ECMD_ONE   = 1,   ///< use first line
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
DLLEXPORT void do_ascii(exarg_T *eap);
DLLEXPORT void ex_align(exarg_T *eap);
DLLEXPORT void ex_sort(exarg_T *eap);
DLLEXPORT void ex_uniq(exarg_T *eap);
DLLEXPORT int do_move(linenr_T line1, linenr_T line2, linenr_T dest);
DLLEXPORT void ex_copy(linenr_T line1, linenr_T line2, linenr_T n);
DLLEXPORT void free_prev_shellcmd(void);
DLLEXPORT void do_bang(int addr_count, exarg_T *eap, bool forceit, bool do_in, bool do_out)
  FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void do_shell(char *cmd, int flags);
DLLEXPORT char *make_filter_cmd(char *cmd, char *itmp, char *otmp, bool do_in);
DLLEXPORT void append_redir(char *const buf, const size_t buflen, const char *const opt,
                            const char *const fname);
DLLEXPORT void print_line_no_prefix(linenr_T lnum, bool use_number, bool list);
DLLEXPORT void print_line(linenr_T lnum, bool use_number, bool list, bool first);
DLLEXPORT int rename_buffer(char *new_fname);
DLLEXPORT void ex_file(exarg_T *eap);
DLLEXPORT void ex_update(exarg_T *eap);
DLLEXPORT void ex_write(exarg_T *eap);
DLLEXPORT int do_write(exarg_T *eap);
DLLEXPORT int check_overwrite(exarg_T *eap, buf_T *buf, char *fname, char *ffname, bool other);
DLLEXPORT void ex_wnext(exarg_T *eap);
DLLEXPORT void do_wqall(exarg_T *eap);
DLLEXPORT int getfile(int fnum, char *ffname_arg, char *sfname_arg, bool setpm, linenr_T lnum,
                      bool forceit);
DLLEXPORT bool set_swapcommand(char *command, linenr_T newlnum);
DLLEXPORT int do_ecmd(int fnum, char *ffname, char *sfname, exarg_T *eap, linenr_T newlnum,
                      int flags, win_T *oldwin);
DLLEXPORT void ex_append(exarg_T *eap);
DLLEXPORT void ex_change(exarg_T *eap);
DLLEXPORT void ex_z(exarg_T *eap);
DLLEXPORT bool check_secure(void);
DLLEXPORT void sub_get_replacement(SubReplacementString *const ret_sub) FUNC_ATTR_NONNULL_ALL;
DLLEXPORT void sub_set_replacement(SubReplacementString sub);
DLLEXPORT bool do_sub_msg(bool count_only);
DLLEXPORT void ex_global(exarg_T *eap);
DLLEXPORT void global_exe(char *cmd);
DLLEXPORT void free_old_sub(void);
DLLEXPORT bool prepare_tagpreview(bool undo_sync, bool use_previewpopup);
DLLEXPORT void ex_substitute(exarg_T *eap);
DLLEXPORT int ex_substitute_preview(exarg_T *eap, int cmdpreview_ns, handle_T cmdpreview_bufnr);
DLLEXPORT char *skip_vimgrep_pat(char *p, char **s, int *flags);
DLLEXPORT void ex_oldfiles(exarg_T *eap);
#include "nvim/func_attr.h"
