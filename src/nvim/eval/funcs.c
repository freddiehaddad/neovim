// funcs.c: C generated function table and va_list wrapper for eval/funcs.rs

#include <math.h>
#include <stdarg.h>
#include <stddef.h>
#include <string.h>

#include "auto/config.h"
#include "nvim/api/private/dispatch.h"
#include "nvim/eval/funcs.h"
#include "nvim/eval/typval_defs.h"
#include "nvim/macros_defs.h"
#include "nvim/os/os_defs.h"
#include "nvim/strings.h"
#include "nvim/types_defs.h"

#include "eval/funcs.c.generated.h"

void float_op_wrapper(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void api_wrapper(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void lua_wrapper(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_abs(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_and(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_api_info(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_atan2(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_byte2line(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_call(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_changenr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_chanclose(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_chansend(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_char2nr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_charcol(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_col(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_confirm(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_copy(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_cursor(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_debugbreak(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_deepcopy(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_dictwatcheradd(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_dictwatcherdel(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_did_filetype(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_empty(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_escape(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getenv(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_eval(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_eventhandler(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_execute(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_exists(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_expand(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_menu_get(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_expandcmd(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_flatten(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_flattennew(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_feedkeys(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_float2nr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_fmod(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_fnameescape(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_foreground(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_funcref(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_function(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_garbagecollect(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_get(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getchangelist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getcharpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getcharsearch(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getfontname(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getjumplist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getmarklist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getpid(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getcurpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getcursorcharpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getregion(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getregionpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getreg(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getregtype(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_gettagstack(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_wait(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_gettext(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_has(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_hlID(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_hlexists(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_index(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_indexof(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_input(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_inputdialog(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_inputlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_inputrestore(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_inputsave(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_inputsecret(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_interrupt(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_invert(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_islocked(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_isinf(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_isnan(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_id(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_jobpid(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_jobresize(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_jobwait(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_json_decode(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_json_encode(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_keytrans(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_len(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_libcall(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_libcallnr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_line(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_line2byte(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_localtime(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_luaeval(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_matchbufline(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_match(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_matchend(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_matchlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_matchstr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_matchstrlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_matchstrpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_max(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_min(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_mode(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_state(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_msgpackdump(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_msgpackparse(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_nextnonblank(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_nr2char(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_or(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_pow(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_prevnonblank(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_printf(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_pum_getpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_pumvisible(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_py3eval(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_rand(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_srand(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_perleval(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_rubyeval(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_range(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_getreginfo(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_reg_executing(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_reg_recording(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_reg_recorded(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_reltime(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_reltimestr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_repeat(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_reduce(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_rpcnotify(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_rpcrequest(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_screenattr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_screenchar(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_screenchars(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_screencol(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_screenrow(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_screenstring(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_search(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_searchdecl(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_searchpair(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_searchpairpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_searchpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_serverlist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_serverstart(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_serverstop(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_setcharpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_setcharsearch(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_setcursorcharpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_setenv(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_setfperm(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_setpos(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_setreg(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_settagstack(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_sha256(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_shellescape(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_shiftwidth(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_sockconnect(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_stdioopen(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_reltimefloat(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_soundfold(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_spellbadword(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_spellsuggest(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_split(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_stdpath(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_str2float(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_strftime(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_strptime(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_submatch(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_substitute(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_swapfilelist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_swapinfo(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_swapname(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_synID(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_synIDattr(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_synIDtrans(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_synconcealed(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_synstack(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_tabpagebuflist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_tagfiles(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_taglist(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_timer_info(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_timer_pause(typval_T *argvars, typval_T *unused, EvalFuncData fptr);
void f_timer_start(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_timer_stop(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_timer_stopall(typval_T *argvars, typval_T *unused, EvalFuncData fptr);
void f_type(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_virtcol(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_visualmode(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_wildmenumode(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_windowsversion(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_wordcount(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);
void f_xor(typval_T *argvars, typval_T *rettv, EvalFuncData fptr);

#ifdef _MSC_VER
// This prevents MSVC from replacing the functions with intrinsics,
// and causing errors when trying to get their addresses in funcs.generated.h
# pragma function(ceil)
# pragma function(floor)
#endif

PRAGMA_DIAG_PUSH_IGNORE_MISSING_PROTOTYPES
PRAGMA_DIAG_PUSH_IGNORE_IMPLICIT_FALLTHROUGH
#include "funcs.generated.h"

PRAGMA_DIAG_POP
PRAGMA_DIAG_POP

/// Dummy va_list for passing to vim_vsnprintf_typval
static va_list dummy_ap;

const char *const eval_has_list[] = {
#ifdef __ANDROID__
  "android",
#endif
#if defined(BSD) && !defined(__APPLE__) && !defined(__GNU__)
  "bsd",
#endif
#ifdef __GNU__
  "hurd",
#endif
#ifdef __linux__
  "linux",
#endif
#ifdef SUN_SYSTEM
  "sun",
#endif
#ifdef __TERMUX__
  "termux",
#endif
#ifdef UNIX
  "unix",
#endif
#ifdef MSWIN
  "win32",
#endif
#ifdef _WIN64
  "win64",
#endif
#ifndef CASE_INSENSITIVE_FILENAME
  "fname_case",
#endif
#ifdef HAVE_ACL
  "acl",
#endif
  "autochdir",
  "arabic",
  "autocmd",
  "browsefilter",
  "byte_offset",
  "cindent",
  "cmdline_compl",
  "cmdline_hist",
  "cmdwin",
  "comments",
  "conceal",
  "cursorbind",
  "cursorshape",
  "dialog_con",
  "diff",
  "digraphs",
  "eval",
  "ex_extra",
  "extra_search",
  "file_in_path",
  "filterpipe",
  "find_in_path",
  "float",
  "folding",
#ifdef UNIX
  "fork",
#endif
  "gettext",
  "iconv",
  "insert_expand",
  "jumplist",
  "keymap",
  "lambda",
  "langmap",
  "libcall",
  "linebreak",
  "lispindent",
  "listcmds",
  "localmap",
#ifdef __APPLE__
  "mac",
  "macunix",
  "osx",
  "osxdarwin",
#endif
  "menu",
  "mksession",
  "modify_fname",
  "mouse",
  "multi_byte",
  "multi_lang",
  "nanotime",
  "num64",
  "packages",
  "path_extra",
  "persistent_undo",
  "profile",
  "reltime",
  "quickfix",
  "rightleft",
  "scrollbind",
  "showcmd",
  "cmdline_info",
  "shada",
  "signs",
  "smartindent",
  "startuptime",
  "statusline",
  "spell",
  "syntax",
#ifndef UNIX
  "system",
#endif
  "tablineat",
  "tag_binary",
  "termguicolors",
#ifdef HAVE_UNIBILIUM
  "terminfo",
#endif
  "termresponse",
  "textobjects",
  "timers",
  "title",
  "user-commands",
  "user_commands",
  "vartabs",
  "vertsplit",
  "vimscript-1",
  "virtualedit",
  "visual",
  "visualextra",
  "vreplace",
  "wildignore",
  "wildmenu",
  "windows",
  "winaltkeys",
  "writebackup",
#ifdef HAVE_XATTR
  "xattr",
#endif
  "nvim",
  NULL,
};

const EvalFuncDef *eval_func_def_at(int idx)
{
  return &functions[idx];
}

int vim_snprintf_typval_only(char *str, size_t str_m, const char *fmt, typval_T *const tvs)
{
  return vim_vsnprintf_typval(str, str_m, fmt, dummy_ap, tvs);
}
