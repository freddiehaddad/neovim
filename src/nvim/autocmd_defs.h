#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "nvim/buffer_defs.h"
#include "nvim/ex_cmds_defs.h"

typedef enum auto_event {
  EVENT_BUFADD = 0,
  EVENT_BUFCREATE = 1,
  EVENT_BUFDELETE = 2,
  EVENT_BUFENTER = 3,
  EVENT_BUFFILEPOST = 4,
  EVENT_BUFFILEPRE = 5,
  EVENT_BUFHIDDEN = 6,
  EVENT_BUFLEAVE = 7,
  EVENT_BUFNEW = 8,
  EVENT_BUFNEWFILE = 9,
  EVENT_BUFREAD = 10,
  EVENT_BUFREADCMD = 11,
  EVENT_BUFREADPOST = 12,
  EVENT_BUFREADPRE = 13,
  EVENT_BUFUNLOAD = 14,
  EVENT_BUFWINENTER = 15,
  EVENT_BUFWINLEAVE = 16,
  EVENT_BUFWIPEOUT = 17,
  EVENT_BUFWRITE = 18,
  EVENT_BUFWRITECMD = 19,
  EVENT_BUFWRITEPOST = 20,
  EVENT_BUFWRITEPRE = 21,
  EVENT_CHANCLOSE = 22,
  EVENT_CHANINFO = 23,
  EVENT_CHANOPEN = 24,
  EVENT_CMDATOM = 25,
  EVENT_CMDLINECHANGED = 26,
  EVENT_CMDLINEENTER = 27,
  EVENT_CMDLINELEAVE = 28,
  EVENT_CMDLINELEAVEPRE = 29,
  EVENT_CMDUNDEFINED = 30,
  EVENT_CMDWINENTER = 31,
  EVENT_CMDWINLEAVE = 32,
  EVENT_COLORSCHEME = 33,
  EVENT_COLORSCHEMEPRE = 34,
  EVENT_COMPLETECHANGED = 35,
  EVENT_COMPLETEDONE = 36,
  EVENT_COMPLETEDONEPRE = 37,
  EVENT_CURSORHOLD = 38,
  EVENT_CURSORHOLDI = 39,
  EVENT_CURSORMOVED = 40,
  EVENT_CURSORMOVEDC = 41,
  EVENT_CURSORMOVEDI = 42,
  EVENT_DIAGNOSTICCHANGED = 43,
  EVENT_DIFFUPDATED = 44,
  EVENT_DIRCHANGED = 45,
  EVENT_DIRCHANGEDPRE = 46,
  EVENT_ENCODINGCHANGED = 47,
  EVENT_EXITPRE = 48,
  EVENT_FILEAPPENDCMD = 49,
  EVENT_FILEAPPENDPOST = 50,
  EVENT_FILEAPPENDPRE = 51,
  EVENT_FILECHANGEDRO = 52,
  EVENT_FILECHANGEDSHELL = 53,
  EVENT_FILECHANGEDSHELLPOST = 54,
  EVENT_FILEENCODING = 55,
  EVENT_FILEREADCMD = 56,
  EVENT_FILEREADPOST = 57,
  EVENT_FILEREADPRE = 58,
  EVENT_FILETYPE = 59,
  EVENT_FILEWRITECMD = 60,
  EVENT_FILEWRITEPOST = 61,
  EVENT_FILEWRITEPRE = 62,
  EVENT_FILTERREADPOST = 63,
  EVENT_FILTERREADPRE = 64,
  EVENT_FILTERWRITEPOST = 65,
  EVENT_FILTERWRITEPRE = 66,
  EVENT_FOCUSGAINED = 67,
  EVENT_FOCUSLOST = 68,
  EVENT_FUNCUNDEFINED = 69,
  EVENT_GUIENTER = 70,
  EVENT_GUIFAILED = 71,
  EVENT_INSERTCHANGE = 72,
  EVENT_INSERTCHARPRE = 73,
  EVENT_INSERTENTER = 74,
  EVENT_INSERTLEAVE = 75,
  EVENT_INSERTLEAVEPRE = 76,
  EVENT_LSPATTACH = 77,
  EVENT_LSPDETACH = 78,
  EVENT_LSPNOTIFY = 79,
  EVENT_LSPPROGRESS = 80,
  EVENT_LSPREQUEST = 81,
  EVENT_LSPTOKENUPDATE = 82,
  EVENT_MARKSET = 83,
  EVENT_MENUPOPUP = 84,
  EVENT_MODECHANGED = 85,
  EVENT_OPTIONSET = 86,
  EVENT_PACKCHANGED = 87,
  EVENT_PACKCHANGEDPRE = 88,
  EVENT_PROGRESS = 89,
  EVENT_QUICKFIXCMDPOST = 90,
  EVENT_QUICKFIXCMDPRE = 91,
  EVENT_QUITPRE = 92,
  EVENT_RECORDINGENTER = 93,
  EVENT_RECORDINGLEAVE = 94,
  EVENT_REMOTEREPLY = 95,
  EVENT_SAFESTATE = 96,
  EVENT_SEARCHWRAPPED = 97,
  EVENT_SESSIONLOADPOST = 98,
  EVENT_SESSIONLOADPRE = 99,
  EVENT_SESSIONWRITEPOST = 100,
  EVENT_SESSIONWRITEPRE = 101,
  EVENT_SHELLCMDPOST = 102,
  EVENT_SHELLFILTERPOST = 103,
  EVENT_SIGNAL = 104,
  EVENT_SOURCECMD = 105,
  EVENT_SOURCEPOST = 106,
  EVENT_SOURCEPRE = 107,
  EVENT_SPELLFILEMISSING = 108,
  EVENT_STDINREADPOST = 109,
  EVENT_STDINREADPRE = 110,
  EVENT_SWAPEXISTS = 111,
  EVENT_SYNTAX = 112,
  EVENT_TABCLOSED = 113,
  EVENT_TABCLOSEDPRE = 114,
  EVENT_TABENTER = 115,
  EVENT_TABLEAVE = 116,
  EVENT_TABMOVED = 117,
  EVENT_TABNEW = 118,
  EVENT_TABNEWENTERED = 119,
  EVENT_TERMCHANGED = 120,
  EVENT_TERMCLOSE = 121,
  EVENT_TERMENTER = 122,
  EVENT_TERMLEAVE = 123,
  EVENT_TERMOPEN = 124,
  EVENT_TERMREQUEST = 125,
  EVENT_TERMRESPONSE = 126,
  EVENT_TEXTCHANGED = 127,
  EVENT_TEXTCHANGEDI = 128,
  EVENT_TEXTCHANGEDP = 129,
  EVENT_TEXTCHANGEDT = 130,
  EVENT_TEXTPUTPOST = 131,
  EVENT_TEXTPUTPRE = 132,
  EVENT_TEXTYANKPOST = 133,
  EVENT_UIENTER = 134,
  EVENT_UILEAVE = 135,
  EVENT_USER = 136,
  EVENT_VIMENTER = 137,
  EVENT_VIMLEAVE = 138,
  EVENT_VIMLEAVEPRE = 139,
  EVENT_VIMRESIZED = 140,
  EVENT_VIMRESUME = 141,
  EVENT_VIMSUSPEND = 142,
  EVENT_WINCLOSED = 143,
  EVENT_WINENTER = 144,
  EVENT_WINLEAVE = 145,
  EVENT_WINNEW = 146,
  EVENT_WINNEWPRE = 147,
  EVENT_WINRESIZED = 148,
  EVENT_WINSCROLLED = 149,
  NUM_EVENTS = 150,
} event_T;

typedef struct {
  size_t refcount;          ///< Reference count (freed when reaches zero)
  char *pat;                ///< Pattern as typed
  regprog_T *reg_prog;      ///< Compiled regprog for pattern
  int group;                ///< Group ID
  int patlen;               ///< strlen() of pat
  int buflocal_nr;          ///< !=0 for buffer-local AutoPat
  char allow_dirs;          ///< Pattern may match whole path
} AutoPat;

typedef struct {
  AutoPat *pat;             ///< Pattern reference (NULL when autocmd was removed)
  int64_t id;               ///< ID used for uniquely tracking an autocmd
  char *desc;               ///< Description for the autocmd
  char *handler_cmd;        ///< Handler Ex command (NULL if handler is a function).
  Callback handler_fn;      ///< Handler callback (ignored if `handler_cmd` is not NULL).
  sctx_T script_ctx;        ///< Script context where it is defined
  bool once;                ///< "One shot": removed after execution
  bool nested;              ///< If autocommands nest here
} AutoCmd;

/// Struct used to keep status while executing autocommands for an event.
typedef struct AutoPatCmd_S AutoPatCmd;
struct AutoPatCmd_S {
  AutoPat *lastpat;         ///< Last matched AutoPat
  size_t auidx;             ///< Current autocmd index to execute
  size_t ausize;            ///< Saved AutoCmd vector size
  char *afile_orig;         ///< Unexpanded <afile>
  char *fname;              ///< Fname to match with
  char *sfname;             ///< Sfname to match with
  char *tail;               ///< Tail of fname
  int group;                ///< Group being used
  event_T event;            ///< Current event
  sctx_T script_ctx;        ///< Script context where it is defined
  int arg_bufnr;            ///< Initially equal to <abuf>, set to zero when buf is deleted
  Object *data;             ///< Arbitrary data
  AutoPatCmd *next;         ///< Chain of active apc-s for auto-invalidation
};

typedef kvec_t(AutoCmd) AutoCmdVec;

typedef struct {
  event_T event;
  char *fname;
  char *fname_io;
  Buffer buf;
  int group;
  exarg_T *eap;
  Object *data;
} AutoCmdEvent;  // Used for "deferred" events, but can represent any event.
