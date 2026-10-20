#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "nvim/eval/typval_defs.h"
#include "nvim/ex_eval_defs.h"
#include "nvim/os/time_defs.h"
#include "nvim/regexp_defs.h"

typedef enum CMD_index {
  CMD_append,
  CMD_abbreviate,
  CMD_abclear,
  CMD_aboveleft,
  CMD_all,
  CMD_amenu,
  CMD_anoremenu,
  CMD_args,
  CMD_argadd,
  CMD_argdelete,
  CMD_argdo,
  CMD_argdedupe,
  CMD_argedit,
  CMD_argglobal,
  CMD_arglocal,
  CMD_argument,
  CMD_ascii,
  CMD_autocmd,
  CMD_augroup,
  CMD_aunmenu,
  CMD_buffer,
  CMD_bNext,
  CMD_ball,
  CMD_badd,
  CMD_balt,
  CMD_bcd,
  CMD_bchdir,
  CMD_bdelete,
  CMD_belowright,
  CMD_bfirst,
  CMD_blast,
  CMD_bmodified,
  CMD_bnext,
  CMD_botright,
  CMD_bprevious,
  CMD_brewind,
  CMD_break,
  CMD_breakadd,
  CMD_breakdel,
  CMD_breaklist,
  CMD_browse,
  CMD_buffers,
  CMD_bufdo,
  CMD_bunload,
  CMD_bwipeout,
  CMD_change,
  CMD_cNext,
  CMD_cNfile,
  CMD_cabbrev,
  CMD_cabclear,
  CMD_cabove,
  CMD_caddbuffer,
  CMD_caddexpr,
  CMD_caddfile,
  CMD_cafter,
  CMD_call,
  CMD_catch,
  CMD_cbuffer,
  CMD_cbefore,
  CMD_cbelow,
  CMD_cbottom,
  CMD_cc,
  CMD_cclose,
  CMD_cd,
  CMD_cdo,
  CMD_center,
  CMD_cexpr,
  CMD_cfile,
  CMD_cfdo,
  CMD_cfirst,
  CMD_cgetfile,
  CMD_cgetbuffer,
  CMD_cgetexpr,
  CMD_chdir,
  CMD_changes,
  CMD_checkhealth,
  CMD_checkpath,
  CMD_checktime,
  CMD_chistory,
  CMD_clist,
  CMD_clast,
  CMD_close,
  CMD_clearjumps,
  CMD_cmap,
  CMD_cmapclear,
  CMD_cmenu,
  CMD_cnext,
  CMD_cnewer,
  CMD_cnfile,
  CMD_cnoremap,
  CMD_cnoreabbrev,
  CMD_cnoremenu,
  CMD_copy,
  CMD_colder,
  CMD_colorscheme,
  CMD_command,
  CMD_comclear,
  CMD_compiler,
  CMD_continue,
  CMD_confirm,
  CMD_connect,
  CMD_const,
  CMD_copen,
  CMD_cprevious,
  CMD_cpfile,
  CMD_cquit,
  CMD_crewind,
  CMD_cunmap,
  CMD_cunabbrev,
  CMD_cunmenu,
  CMD_cwindow,
  CMD_delete,
  CMD_delmarks,
  CMD_debug,
  CMD_debuggreedy,
  CMD_defer,
  CMD_delcommand,
  CMD_delfunction,
  CMD_detach,
  CMD_display,
  CMD_diffupdate,
  CMD_diffget,
  CMD_diffoff,
  CMD_diffpatch,
  CMD_diffput,
  CMD_diffsplit,
  CMD_diffthis,
  CMD_digraphs,
  CMD_djump,
  CMD_dlist,
  CMD_doautocmd,
  CMD_doautoall,
  CMD_drop,
  CMD_dsearch,
  CMD_dsplit,
  CMD_edit,
  CMD_earlier,
  CMD_echo,
  CMD_echoerr,
  CMD_echohl,
  CMD_echomsg,
  CMD_echon,
  CMD_else,
  CMD_elseif,
  CMD_emenu,
  CMD_endif,
  CMD_endfunction,
  CMD_endfor,
  CMD_endtry,
  CMD_endwhile,
  CMD_enew,
  CMD_eval,
  CMD_ex,
  CMD_execute,
  CMD_exit,
  CMD_exmode,
  CMD_exusage,
  CMD_file,
  CMD_files,
  CMD_filetype,
  CMD_filter,
  CMD_find,
  CMD_finally,
  CMD_finish,
  CMD_first,
  CMD_fold,
  CMD_foldclose,
  CMD_folddoopen,
  CMD_folddoclosed,
  CMD_foldopen,
  CMD_for,
  CMD_function,
  CMD_fclose,
  CMD_global,
  CMD_goto,
  CMD_grep,
  CMD_grepadd,
  CMD_gui,
  CMD_gvim,
  CMD_help,
  CMD_helpclose,
  CMD_helpgrep,
  CMD_helptags,
  CMD_highlight,
  CMD_hide,
  CMD_history,
  CMD_horizontal,
  CMD_insert,
  CMD_iabbrev,
  CMD_iabclear,
  CMD_if,
  CMD_ijump,
  CMD_ilist,
  CMD_imap,
  CMD_imapclear,
  CMD_imenu,
  CMD_inoremap,
  CMD_inoreabbrev,
  CMD_inoremenu,
  CMD_intro,
  CMD_iput,
  CMD_isearch,
  CMD_isplit,
  CMD_iunmap,
  CMD_iunabbrev,
  CMD_iunmenu,
  CMD_join,
  CMD_jumps,
  CMD_k,
  CMD_keepmarks,
  CMD_keepjumps,
  CMD_keeppatterns,
  CMD_keepalt,
  CMD_list,
  CMD_lNext,
  CMD_lNfile,
  CMD_last,
  CMD_labove,
  CMD_language,
  CMD_laddexpr,
  CMD_laddbuffer,
  CMD_laddfile,
  CMD_lafter,
  CMD_later,
  CMD_lbuffer,
  CMD_lbefore,
  CMD_lbelow,
  CMD_lbottom,
  CMD_lcd,
  CMD_lchdir,
  CMD_lclose,
  CMD_ldo,
  CMD_left,
  CMD_leftabove,
  CMD_let,
  CMD_lexpr,
  CMD_lfile,
  CMD_lfdo,
  CMD_lfirst,
  CMD_lgetfile,
  CMD_lgetbuffer,
  CMD_lgetexpr,
  CMD_lgrep,
  CMD_lgrepadd,
  CMD_lhelpgrep,
  CMD_lhistory,
  CMD_ll,
  CMD_llast,
  CMD_llist,
  CMD_lmap,
  CMD_lmapclear,
  CMD_lmake,
  CMD_lnoremap,
  CMD_lnext,
  CMD_lnewer,
  CMD_lnfile,
  CMD_loadview,
  CMD_loadkeymap,
  CMD_lockmarks,
  CMD_lockvar,
  CMD_log,
  CMD_lolder,
  CMD_lopen,
  CMD_lprevious,
  CMD_lpfile,
  CMD_lrewind,
  CMD_ltag,
  CMD_lunmap,
  CMD_lua,
  CMD_luado,
  CMD_luafile,
  CMD_lvimgrep,
  CMD_lvimgrepadd,
  CMD_lwindow,
  CMD_ls,
  CMD_lsp,
  CMD_move,
  CMD_mark,
  CMD_make,
  CMD_map,
  CMD_mapclear,
  CMD_marks,
  CMD_match,
  CMD_menu,
  CMD_menutranslate,
  CMD_messages,
  CMD_mkexrc,
  CMD_mksession,
  CMD_mkspell,
  CMD_mkvimrc,
  CMD_mkview,
  CMD_mode,
  CMD_mzscheme,
  CMD_mzfile,
  CMD_next,
  CMD_new,
  CMD_nmap,
  CMD_nmapclear,
  CMD_nmenu,
  CMD_nnoremap,
  CMD_nnoremenu,
  CMD_noremap,
  CMD_noautocmd,
  CMD_nohlsearch,
  CMD_noreabbrev,
  CMD_noremenu,
  CMD_noswapfile,
  CMD_normal,
  CMD_number,
  CMD_nunmap,
  CMD_nunmenu,
  CMD_oldfiles,
  CMD_omap,
  CMD_omapclear,
  CMD_omenu,
  CMD_only,
  CMD_onoremap,
  CMD_onoremenu,
  CMD_options,
  CMD_ounmap,
  CMD_ounmenu,
  CMD_ownsyntax,
  CMD_print,
  CMD_packadd,
  CMD_packdel,
  CMD_packloadall,
  CMD_packupdate,
  CMD_pbuffer,
  CMD_pclose,
  CMD_perl,
  CMD_perldo,
  CMD_perlfile,
  CMD_pedit,
  CMD_pop,
  CMD_popup,
  CMD_ppop,
  CMD_preserve,
  CMD_previous,
  CMD_profile,
  CMD_profdel,
  CMD_psearch,
  CMD_ptag,
  CMD_ptNext,
  CMD_ptfirst,
  CMD_ptjump,
  CMD_ptlast,
  CMD_ptnext,
  CMD_ptprevious,
  CMD_ptrewind,
  CMD_ptselect,
  CMD_put,
  CMD_pwd,
  CMD_python,
  CMD_pydo,
  CMD_pyfile,
  CMD_py3,
  CMD_py3do,
  CMD_python3,
  CMD_py3file,
  CMD_pyx,
  CMD_pyxdo,
  CMD_pythonx,
  CMD_pyxfile,
  CMD_quit,
  CMD_quitall,
  CMD_qall,
  CMD_read,
  CMD_recover,
  CMD_redo,
  CMD_redir,
  CMD_redraw,
  CMD_redrawstatus,
  CMD_redrawtabline,
  CMD_registers,
  CMD_resize,
  CMD_restart,
  CMD_retab,
  CMD_return,
  CMD_rewind,
  CMD_right,
  CMD_rightbelow,
  CMD_rshada,
  CMD_runtime,
  CMD_rundo,
  CMD_ruby,
  CMD_rubydo,
  CMD_rubyfile,
  CMD_rviminfo,
  CMD_substitute,
  CMD_sNext,
  CMD_sargument,
  CMD_sall,
  CMD_sandbox,
  CMD_saveas,
  CMD_sbuffer,
  CMD_sbNext,
  CMD_sball,
  CMD_sbfirst,
  CMD_sblast,
  CMD_sbmodified,
  CMD_sbnext,
  CMD_sbprevious,
  CMD_sbrewind,
  CMD_scriptnames,
  CMD_scriptencoding,
  CMD_set,
  CMD_setfiletype,
  CMD_setglobal,
  CMD_setlocal,
  CMD_sfind,
  CMD_sfirst,
  CMD_simalt,
  CMD_sign,
  CMD_silent,
  CMD_sleep,
  CMD_slast,
  CMD_smagic,
  CMD_smap,
  CMD_smapclear,
  CMD_smenu,
  CMD_snext,
  CMD_snomagic,
  CMD_snoremap,
  CMD_snoremenu,
  CMD_source,
  CMD_sort,
  CMD_split,
  CMD_spellgood,
  CMD_spelldump,
  CMD_spellinfo,
  CMD_spellrepall,
  CMD_spellrare,
  CMD_spellundo,
  CMD_spellwrong,
  CMD_sprevious,
  CMD_srewind,
  CMD_stop,
  CMD_stag,
  CMD_startinsert,
  CMD_startgreplace,
  CMD_startreplace,
  CMD_stopinsert,
  CMD_stjump,
  CMD_stselect,
  CMD_sunhide,
  CMD_sunmap,
  CMD_sunmenu,
  CMD_suspend,
  CMD_sview,
  CMD_swapname,
  CMD_syntax,
  CMD_syntime,
  CMD_syncbind,
  CMD_t,
  CMD_tcd,
  CMD_tchdir,
  CMD_tNext,
  CMD_tag,
  CMD_tags,
  CMD_tab,
  CMD_tabclose,
  CMD_tabdo,
  CMD_tabedit,
  CMD_tabfind,
  CMD_tabfirst,
  CMD_tabmove,
  CMD_tablast,
  CMD_tabnext,
  CMD_tabnew,
  CMD_tabonly,
  CMD_tabprevious,
  CMD_tabNext,
  CMD_tabrewind,
  CMD_tabs,
  CMD_tcl,
  CMD_tcldo,
  CMD_tclfile,
  CMD_terminal,
  CMD_tfirst,
  CMD_throw,
  CMD_tjump,
  CMD_tlast,
  CMD_tlmenu,
  CMD_tlnoremenu,
  CMD_tlunmenu,
  CMD_tmenu,
  CMD_tmap,
  CMD_tmapclear,
  CMD_tnext,
  CMD_tnoremap,
  CMD_topleft,
  CMD_tprevious,
  CMD_trewind,
  CMD_trust,
  CMD_try,
  CMD_tselect,
  CMD_tunmenu,
  CMD_tunmap,
  CMD_undo,
  CMD_undojoin,
  CMD_undolist,
  CMD_unabbreviate,
  CMD_unhide,
  CMD_uniq,
  CMD_unlet,
  CMD_unlockvar,
  CMD_unmap,
  CMD_unmenu,
  CMD_unsilent,
  CMD_update,
  CMD_uptime,
  CMD_vglobal,
  CMD_version,
  CMD_verbose,
  CMD_vertical,
  CMD_visual,
  CMD_view,
  CMD_vimgrep,
  CMD_vimgrepadd,
  CMD_viusage,
  CMD_vmap,
  CMD_vmapclear,
  CMD_vmenu,
  CMD_vnoremap,
  CMD_vnew,
  CMD_vnoremenu,
  CMD_vsplit,
  CMD_vunmap,
  CMD_vunmenu,
  CMD_write,
  CMD_wNext,
  CMD_wall,
  CMD_while,
  CMD_winsize,
  CMD_wincmd,
  CMD_windo,
  CMD_winpos,
  CMD_wnext,
  CMD_wprevious,
  CMD_wq,
  CMD_wqall,
  CMD_wshada,
  CMD_wundo,
  CMD_wviminfo,
  CMD_xit,
  CMD_xall,
  CMD_xmap,
  CMD_xmapclear,
  CMD_xmenu,
  CMD_xnoremap,
  CMD_xnoremenu,
  CMD_xunmap,
  CMD_xunmenu,
  CMD_yank,
  CMD_z,
  CMD_bang,
  CMD_pound,
  CMD_and,
  CMD_lshift,
  CMD_equal,
  CMD_rshift,
  CMD_at,
  CMD_tilde,
  CMD_Next,
  CMD_SIZE,
  CMD_USER = -1,
  CMD_USER_BUF = -2,
} cmdidx_T;

// When adding an Ex command:
// 1. Add an entry to the table in src/nvim/ex_cmds.lua.  Keep it sorted on the
//    shortest version of the command name that works.  If it doesn't start with
//    a lower case letter, add it at the end.
//
//    Each table entry is a table with the following keys:
//
//      Key     | Description
//      ------- | -------------------------------------------------------------
//      command | Name of the command. Required.
//      enum    | Name of the enum entry. If not set defaults to CMD_{command}.
//      flags   | A set of the flags from below list joined by bitwise or.
//      func    | Name of the function containing the implementation.
//
//    Referenced function should be either non-static one or defined in
//    ex_docmd.c and be coercible to ex_func_T type from below.
//
//    All keys not described in the above table are reserved for future use.
//
// 2. Add an entry in the index for Ex commands at ":help ex-cmd-index".
// 3. Add documentation in ../doc/xxx.txt.  Add a tag for both the short and
//    long name of the command.

#define EX_RANGE           0x001U  // allow a linespecs
#define EX_BANG            0x002U  // allow a ! after the command name
#define EX_EXTRA           0x004U  // allow extra args after command name
#define EX_XFILE           0x008U  // expand wildcards in extra part
#define EX_NOSPC           0x010U  // extra part is a single argument (no split on
                                   // whitespace)
#define EX_DFLALL          0x020U  // default file range is 1,$
#define EX_WHOLEFOLD       0x040U  // extend range to include whole fold also
                                   // when less than two numbers given
#define EX_NEEDARG         0x080U  // argument required
#define EX_TRLBAR          0x100U  // check for trailing vertical bar
#define EX_REGSTR          0x200U  // allow "x for register designation
#define EX_COUNT           0x400U  // allow count in argument, after command
#define EX_NOTRLCOM        0x800U  // no trailing comment allowed
#define EX_ZEROR          0x1000U  // zero line number allowed
#define EX_CTRLV          0x2000U  // do not remove CTRL-V from argument
#define EX_CMDARG         0x4000U  // allow "+command" argument
#define EX_BUFNAME        0x8000U  // accepts buffer name
#define EX_BUFUNL        0x10000U  // accepts unlisted buffer too
#define EX_ARGOPT        0x20000U  // allow "++opt=val" argument
#define EX_SBOXOK        0x40000U  // allowed in the sandbox
#define EX_BUFLOCK_OK    0x80000U  // Command is allowed when curbuf is `b_ro_locked` (e.g. during
                                   // a quickfix or diff critical section). Legacy name: EX_CMDWIN.
                                   // Implies EX_LOCK_OK.
#define EX_MODIFY       0x100000U  // forbidden in non-'modifiable' buffer
#define EX_FLAGS        0x200000U  // allow flags after count in argument
#define EX_LOCK_OK     0x1000000U  // Command allowed when |textlock| is set. EX_BUFLOCK_OK is per-buffer.
#define EX_KEEPSCRIPT  0x4000000U  // keep sctx of where command was invoked
#define EX_PREVIEW     0x8000000U  // allow incremental command preview
#define EX_ARGSPACE   0x40000000U  // completion: keep spaces in arg lead
#define EX_FILES (EX_XFILE | EX_EXTRA)  // multiple extra files allowed
#define EX_FILE1 (EX_FILES | EX_NOSPC)  // 1 file, defaults to current file
#define EX_WORD1 (EX_EXTRA | EX_NOSPC)  // one extra word allowed

/// values for cmd_addr_type
typedef enum {
  ADDR_LINES,           ///< buffer line numbers
  ADDR_WINDOWS,         ///< window number
  ADDR_ARGUMENTS,       ///< argument number
  ADDR_LOADED_BUFFERS,  ///< buffer number of loaded buffer
  ADDR_BUFFERS,         ///< buffer number
  ADDR_TABS,            ///< tab page number
  ADDR_TABS_RELATIVE,   ///< Tab page that only relative
  ADDR_QUICKFIX_VALID,  ///< quickfix list valid entry number
  ADDR_QUICKFIX,        ///< quickfix list entry number
  ADDR_UNSIGNED,        ///< positive count or zero, defaults to 1
  ADDR_OTHER,           ///< something else, use line number for '$', '%', etc.
  ADDR_NONE,            ///< no range used
} cmd_addr_T;

typedef struct exarg exarg_T;

// behavior for bad character, "++bad=" argument
#define BAD_REPLACE     '?'     // replace it with '?' (default)
#define BAD_KEEP        (-1)    // leave it
#define BAD_DROP        (-2)    // erase it

typedef void (*ex_func_T)(exarg_T *eap);
typedef int (*ex_preview_func_T)(exarg_T *eap, int cmdpreview_ns, handle_T cmdpreview_bufnr);

typedef char *(*LineGetter)(int, void *, int, bool);

/// Structure for command definition.
typedef struct {
  char *cmd_name;                      ///< Name of the command.
  ex_func_T cmd_func;                  ///< Function with implementation of this command.
  ex_preview_func_T cmd_preview_func;  ///< Preview callback function of this command.
  uint32_t cmd_argt;                   ///< Relevant flags from the declared above.
  cmd_addr_T cmd_addr_type;            ///< Flag for address type.
} CommandDefinition;

/// Arguments used for Ex commands.
struct exarg {
  char *arg;                    ///< argument of the command
  char **args;                  ///< starting position of command arguments
  size_t *arglens;              ///< length of command arguments
  size_t argc;                  ///< number of command arguments
  char *nextcmd;                ///< next command (NULL if none)
  char *cmd;                    ///< the name of the command (except for :make)
  char **cmdlinep;              ///< pointer to pointer of allocated cmdline
  char *cmdline_tofree;         ///< free later
  cmdidx_T cmdidx;              ///< the index for the command
  uint32_t argt;                ///< flags for the command
  int skip;                     ///< don't execute the command, only parse it
  int forceit;                  ///< true if ! present
  int addr_count;               ///< the number of addresses given
  linenr_T line1;               ///< the first line number
  linenr_T line2;               ///< the second line number or count
  cmd_addr_T addr_type;         ///< type of the count/range
  int flags;                    ///< extra flags after count: EXFLAG_
  char *do_ecmd_cmd;            ///< +command arg to be used in edited file
  linenr_T do_ecmd_lnum;        ///< the line number in an edited file
  int append;                   ///< true with ":w >>file" command
  int usefilter;                ///< true with ":w !command" and ":r!command"
  int amount;                   ///< number of '>' or '<' for shift command
  int regname;                  ///< register name (NUL if none)
  int force_bin;                ///< 0, FORCE_BIN or FORCE_NOBIN
  int read_edit;                ///< ++edit argument
  int mkdir_p;                  ///< ++p argument
  int force_ff;                 ///< ++ff= argument (first char of argument)
  int force_enc;                ///< ++enc= argument (index in cmd[])
  int bad_char;                 ///< BAD_KEEP, BAD_DROP or replacement byte
  int useridx;                  ///< user command index
  char *errmsg;                 ///< returned error message
  LineGetter ea_getline;        ///< function used to get the next line
  void *cookie;                 ///< argument for ea_getline()
  cstack_T *cstack;             ///< condition stack for ":if" etc.
  struct {                      ///< special char handling in command args
    bool file;
    bool bar;
  } magic;
};

#define FORCE_BIN 1             // ":edit ++bin file"
#define FORCE_NOBIN 2           // ":edit ++nobin file"

// Values for "flags"
#define EXFLAG_LIST     0x01    // 'l': list
#define EXFLAG_NR       0x02    // '#': number
#define EXFLAG_PRINT    0x04    // 'p': print

enum {
  CMOD_SANDBOX      = 0x0001,  ///< ":sandbox"
  CMOD_SILENT       = 0x0002,  ///< ":silent"
  CMOD_ERRSILENT    = 0x0004,  ///< ":silent!"
  CMOD_UNSILENT     = 0x0008,  ///< ":unsilent"
  CMOD_NOAUTOCMD    = 0x0010,  ///< ":noautocmd"
  CMOD_HIDE         = 0x0020,  ///< ":hide"
  CMOD_BROWSE       = 0x0040,  ///< ":browse" - invoke file dialog
  CMOD_CONFIRM      = 0x0080,  ///< ":confirm" - invoke yes/no dialog
  CMOD_KEEPALT      = 0x0100,  ///< ":keepalt"
  CMOD_KEEPMARKS    = 0x0200,  ///< ":keepmarks"
  CMOD_KEEPJUMPS    = 0x0400,  ///< ":keepjumps"
  CMOD_LOCKMARKS    = 0x0800,  ///< ":lockmarks"
  CMOD_KEEPPATTERNS = 0x1000,  ///< ":keeppatterns"
  CMOD_NOSWAPFILE   = 0x2000,  ///< ":noswapfile"
};

/// Command modifiers ":vertical", ":browse", ":confirm", ":hide", etc. set a
/// flag.  This needs to be saved for recursive commands, put them in a
/// structure for easy manipulation.
typedef struct {
  int cmod_flags;  ///< CMOD_ flags

  int cmod_split;  ///< flags for win_split()
  int cmod_tab;  ///< > 0 when ":tab" was used
  char *cmod_filter_pat;
  regmatch_T cmod_filter_regmatch;  ///< set by :filter /pat/
  bool cmod_filter_force;  ///< set for :filter!

  int cmod_verbose;  ///< 0 if not set, > 0 to set 'verbose' to cmod_verbose - 1

  // values for undo_cmdmod()
  char *cmod_save_ei;  ///< saved value of 'eventignore'
  int cmod_did_sandbox;  ///< set when "sandbox" was incremented
  OptInt cmod_verbose_save;  ///< if 'verbose' was set: value of p_verbose plus one
  int cmod_save_msg_silent;  ///< if non-zero: saved value of msg_silent + 1
  int cmod_save_msg_scroll;  ///< for restoring msg_scroll
  int cmod_did_esilent;  ///< incremented when emsg_silent is
} cmdmod_T;

/// Previous :substitute replacement string definition
typedef struct {
  char *sub;            ///< Previous replacement string.
  Timestamp timestamp;  ///< Time when it was last set.
  AdditionalData *additional_data;  ///< Additional data left from ShaDa file.
} SubReplacementString;
