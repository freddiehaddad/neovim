// Make sure extern symbols are exported on Windows
#ifdef WIN32
# define EXTERN __declspec(dllexport)
#else
# define EXTERN
#endif
#include <assert.h>
#include <lauxlib.h>  // IWYU pragma: keep
#include <lua.h>  // IWYU pragma: keep
#include <lualib.h>  // IWYU pragma: keep
#include <math.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef ENABLE_ASAN_UBSAN
# include <sanitizer/asan_interface.h>
# ifndef MSWIN
#  include <sanitizer/ubsan_interface.h>
# endif
#endif

#include "auto/config.h"  // IWYU pragma: keep
#include "auto/pathdef.h"
#include "nvim/option_defs.h"
#include "nvim/option_vars.h"

#undef EXTERN
#undef INIT
#define EXTERN extern
#define INIT(...)

#include "auto/versiondef.h"
#include "auto/versiondef_git.h"
#include "nvim/api/private/defs.h"
#include "nvim/api/private/dispatch.h"
#include "nvim/api/private/helpers.h"
#include "nvim/ascii_defs.h"
#include "nvim/assert_defs.h"
#include "nvim/errors.h"  // IWYU pragma: keep
#include "nvim/eval/funcs.h"
#include "nvim/eval/typval_defs.h"
#include "nvim/event/loop.h"
#include "nvim/ex_docmd.h"  // IWYU pragma: keep
#include "nvim/ex_getln.h"  // IWYU pragma: keep
#include "nvim/func_attr.h"  // IWYU pragma: keep
#include "nvim/gettext_defs.h"
#include "nvim/globals.h"  // IWYU pragma: keep
#include "nvim/lua/converter.h"  // IWYU pragma: keep
#include "nvim/lua/executor.h"
#include "nvim/macros_defs.h"
#include "nvim/main.h"
#include "nvim/memory.h"
#include "nvim/memory_defs.h"
#include "nvim/message.h"
#include "nvim/msgpack_rpc/unpacker.h"
#include "nvim/option.h"
#include "nvim/os/os_defs.h"
#include "nvim/strings.h"
#include "nvim/types_defs.h"
#include "nvim/version.h"
#include "nvim/vim_defs.h"

#ifdef BACKSLASH_IN_FILENAME
# include "nvim/arglist.h"
#endif

#ifdef MSWIN
# ifndef _UCRT
#  error UCRT is the only supported C runtime on windows
# endif
#endif

typedef struct {
  char *name;
  const uint8_t *data;
  size_t size;
} ModuleDef;

typedef struct {
  va_list ap_start;
  va_list ap;
} VaState;

const char *did_set_arabic(optset_T *args);
const char *did_set_autochdir(optset_T *args);
const char *did_set_binary(optset_T *args);
const char *did_set_buflisted(optset_T *args);
const char *did_set_cmdheight(optset_T *args);
const char *did_set_diff(optset_T *args);
const char *did_set_eof_eol_fixeol_bomb(optset_T *args);
const char *did_set_equalalways(optset_T *args);
const char *did_set_foldenable(optset_T *args);
const char *did_set_foldlevel(optset_T *args);
const char *did_set_foldminlines(optset_T *args);
const char *did_set_foldnestmax(optset_T *args);
const char *did_set_follow(optset_T *args);
const char *did_set_helpheight(optset_T *args);
const char *did_set_hlsearch(optset_T *args);
const char *did_set_ignorecase(optset_T *args);
const char *did_set_iminsert(optset_T *args);
const char *did_set_langnoremap(optset_T *args);
const char *did_set_langremap(optset_T *args);
const char *did_set_laststatus(optset_T *args);
const char *did_set_lines_or_columns(optset_T *args);
const char *did_set_lisp(optset_T *args);
const char *did_set_modifiable(optset_T *args);
const char *did_set_modified(optset_T *args);
const char *did_set_number_relativenumber(optset_T *args);
const char *did_set_numberwidth(optset_T *args);
const char *did_set_paste(optset_T *args);
const char *validate_previewwindow(optset_T *args);
const char *did_set_pumblend(optset_T *args);
const char *did_set_readonly(optset_T *args);
const char *did_set_scrollback(optset_T *args);
const char *did_set_scrollbind(optset_T *args);
const char *did_set_shiftwidth_tabstop(optset_T *args);
const char *did_set_showtabline(optset_T *args);
const char *did_set_smoothscroll(optset_T *args);
const char *did_set_spell(optset_T *args);
const char *did_set_swapfile(optset_T *args);
const char *did_set_textwidth(optset_T *args);
const char *did_set_title_icon(optset_T *args);
const char *did_set_titlelen(optset_T *args);
const char *did_set_undofile(optset_T *args);
const char *did_set_undolevels(optset_T *args);
const char *did_set_updatecount(optset_T *args);
const char *validate_wildchar(optset_T *args);
const char *did_set_winblend(optset_T *args);
const char *did_set_window(optset_T *args);
const char *did_set_winheight(optset_T *args);
const char *did_set_winwidth(optset_T *args);
const char *did_set_wrap(optset_T *args);
const char *did_set_xhistory(optset_T *args);

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
# pragma function(ceil)
# pragma function(floor)
#endif

#include "api/private/api_metadata.generated.h"
#include "api/private/dispatch_wrappers.generated.h"
#include "lua/vim_module.generated.h"
#include "lua_api_c_bindings.generated.h"  // IWYU pragma: keep
#include "main.c.generated.h"

#include "options.generated.h"
#include "options_map.generated.h"

PRAGMA_DIAG_PUSH_IGNORE_MISSING_PROTOTYPES
PRAGMA_DIAG_PUSH_IGNORE_IMPLICIT_FALLTHROUGH
#include "funcs.generated.h"

PRAGMA_DIAG_POP
PRAGMA_DIAG_POP

// for ":version", ":intro", and "nvim --version"
#ifndef NVIM_VERSION_MEDIUM
# define NVIM_VERSION_MEDIUM "v" STR(NVIM_VERSION_MAJOR) \
  "." STR(NVIM_VERSION_MINOR) "." STR(NVIM_VERSION_PATCH) \
  NVIM_VERSION_PRERELEASE
#endif
#define NVIM_VERSION_LONG "NVIM " NVIM_VERSION_MEDIUM  // NOLINT(bugprone-suspicious-missing-comma)

char *Versions[] = { "8.1", "8.2", "9.0", "9.1", "9.2" };
char *longVersion = NVIM_VERSION_LONG;
char *version_buildtype = "Build type: " NVIM_VERSION_BUILD_TYPE;
#ifndef NDEBUG
char *version_cflags = "Compilation: " NVIM_VERSION_CFLAGS;
#else
char *version_cflags = NULL;
#endif
#ifdef SYS_VIMRC_FILE
const char *version_sys_vimrc_file = SYS_VIMRC_FILE;
#else
const char *version_sys_vimrc_file = NULL;
#endif
const int nvim_version_major = NVIM_VERSION_MAJOR;
const int nvim_version_minor = NVIM_VERSION_MINOR;
const int nvim_version_patch = NVIM_VERSION_PATCH;

Loop main_loop;

#ifdef EXITFREE
const bool main_exitfree = true;
#else
const bool main_exitfree = false;
#endif

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

OptIndex find_option_len(const char *const name, size_t len)
  FUNC_ATTR_NONNULL_ALL
{
  int index = find_option_hash(name, len);
  return index >= 0 ? option_hash_elems[index].opt_idx : kOptInvalid;
}

vimoption_T *get_option(OptIndex opt_idx)
{
  assert(opt_idx != kOptInvalid);
  return &options[opt_idx];
}

const void *nlua_get_builtin_modules(size_t *count)
{
  *count = ARRAY_SIZE(builtin_modules);
  return builtin_modules;
}

MsgpackRpcRequestHandler msgpack_rpc_get_handler_at(int hash)
{
  return method_handlers[hash];
}

const uint8_t *api_get_packed_metadata(size_t *len)
{
  *len = sizeof(packed_api_metadata);
  return packed_api_metadata;
}

void api_set_error(Error *err, ErrorType errType, const char *format, ...)
  FUNC_ATTR_NONNULL_ALL FUNC_ATTR_PRINTF(3, 4)
{
  assert(kErrorTypeNone != errType);
  va_list args1;
  va_list args2;
  va_start(args1, format);
  va_copy(args2, args1);
  int len = vsnprintf(NULL, 0, format, args1);
  va_end(args1);
  assert(len >= 0);
  // Limit error message to 1 MB.
  size_t bufsize = MIN((size_t)len + 1, 1024 * 1024);
  err->msg = xmalloc(bufsize);
  vsnprintf(err->msg, bufsize, format, args2);
  va_end(args2);

  err->type = errType;
}

int vim_vsnprintf_typval_rs(char *str, size_t str_m, const char *fmt, void *va_ptr,
                            typval_T *const tvs);
long long va_state_llong(void *va_ptr);  // NOLINT(runtime/int)
unsigned long va_state_ulong(void *va_ptr);  // NOLINT(runtime/int)
unsigned long long va_state_ullong(void *va_ptr);  // NOLINT(runtime/int)

void va_state_reset(void *va_ptr)
{
  VaState *va = va_ptr;
  va_end(va->ap);
  va_copy(va->ap, va->ap_start);
}

int va_state_int(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, int);
}

long va_state_long(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, long);  // NOLINT(runtime/int)
}

long long va_state_llong(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, long long);  // NOLINT(runtime/int)
}

ptrdiff_t va_state_ptrdiff(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, ptrdiff_t);
}

unsigned va_state_uint(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, unsigned);
}

unsigned long va_state_ulong(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, unsigned long);  // NOLINT(runtime/int)
}

unsigned long long va_state_ullong(void *va_ptr)  // NOLINT(runtime/int)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, unsigned long long);  // NOLINT(runtime/int)
}

size_t va_state_size(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, size_t);
}

const char *va_state_str(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, const char *);
}

void *va_state_ptr(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, void *);
}

double va_state_double(void *va_ptr)
{
  VaState *va = va_ptr;
  return va_arg(va->ap, double);
}

void va_report_null_string(void)
{
#ifdef ENABLE_ASAN_UBSAN
  __sanitizer_print_stack_trace();
  __sanitizer_report_error_summary("SUMMARY: NULL argument passed to %s");
#endif
}

int vim_snprintf(char *str, size_t str_m, const char *fmt, ...)
  FUNC_ATTR_PRINTF(3, 4)
{
  va_list ap;
  va_start(ap, fmt);
  VaState va;
  va_copy(va.ap_start, ap);
  va_copy(va.ap, ap);
  const int str_l = vim_vsnprintf_typval_rs(str, str_m, fmt, &va, NULL);
  va_end(va.ap);
  va_end(va.ap_start);
  va_end(ap);
  return str_l;
}

#ifndef MAKE_LIB
int main(int argc, char **argv)
{
  return nvim_main(argc, argv);
}
#endif

#ifdef ENABLE_ASAN_UBSAN
const char *__ubsan_default_options(void)
{
  return "print_stacktrace=1";
}

const char *__asan_default_options(void)
{
  return "handle_abort=1,handle_sigill=1";
}
#endif
