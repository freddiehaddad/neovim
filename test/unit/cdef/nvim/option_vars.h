#pragma once

#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/option_defs.h"
#include "nvim/os/os_defs.h"
#include "nvim/sign_defs.h"
#include "nvim/statusline_defs.h"
#include "nvim/types_defs.h"

typedef enum {
  kOptBkcFlagYes = 0x01,
  kOptBkcFlagAuto = 0x02,
  kOptBkcFlagNo = 0x04,
  kOptBkcFlagBreaksymlink = 0x08,
  kOptBkcFlagBreakhardlink = 0x10,
} OptBkcFlags;

typedef enum {
  kOptBoFlagAll = 0x01,
  kOptBoFlagBackspace = 0x02,
  kOptBoFlagCursor = 0x04,
  kOptBoFlagComplete = 0x08,
  kOptBoFlagCopy = 0x10,
  kOptBoFlagCtrlg = 0x20,
  kOptBoFlagError = 0x40,
  kOptBoFlagEsc = 0x80,
  kOptBoFlagEx = 0x100,
  kOptBoFlagHangul = 0x200,
  kOptBoFlagInsertmode = 0x400,
  kOptBoFlagLang = 0x800,
  kOptBoFlagMess = 0x1000,
  kOptBoFlagShowmatch = 0x2000,
  kOptBoFlagOperator = 0x4000,
  kOptBoFlagRegister = 0x8000,
  kOptBoFlagShell = 0x10000,
  kOptBoFlagSpell = 0x20000,
  kOptBoFlagTerm = 0x40000,
  kOptBoFlagWildmode = 0x80000,
} OptBoFlags;

typedef enum {
  kOptCmpFlagInternal = 0x01,
  kOptCmpFlagKeepascii = 0x02,
} OptCmpFlags;

typedef enum {
  kOptCbFlagUnnamed = 0x01,
  kOptCbFlagUnnamedplus = 0x02,
} OptCbFlags;

typedef enum {
  kOptCotFlagFuzzy = 0x01,
  kOptCotFlagLongest = 0x02,
  kOptCotFlagMenu = 0x04,
  kOptCotFlagMenuone = 0x08,
  kOptCotFlagNearest = 0x10,
  kOptCotFlagNoinsert = 0x20,
  kOptCotFlagNoselect = 0x40,
  kOptCotFlagNosort = 0x80,
  kOptCotFlagPopup = 0x100,
  kOptCotFlagPreinsert = 0x200,
  kOptCotFlagPreselect = 0x400,
  kOptCotFlagPreview = 0x800,
} OptCotFlags;

enum {
  kCpoAltread = 'a',
  kCpoAltwrite = 'A',
  kCpoBackspace = 'v',
  kCpoBar = 'b',
  kCpoBslash = 'B',
  kCpoBufopt = 's',
  kCpoBufoptglob = 'S',
  kCpoChangew = '_',
  kCpoConcat = 'C',
  kCpoDigraph = 'D',
  kCpoDollar = '$',
  kCpoDottag = 'd',
  kCpoEmptyregion = 'E',
  kCpoEndofsent = 'J',
  kCpoEsc = 'x',
  kCpoExecbuf = 'e',
  kCpoFilter = '!',
  kCpoFnameapp = 'P',
  kCpoFnamer = 'f',
  kCpoFnamew = 'F',
  kCpoFwrite = 'W',
  kCpoIndent = 'I',
  kCpoIntmod = 'i',
  kCpoJoincol = 'q',
  kCpoKeepro = 'Z',
  kCpoKoffset = 'K',
  kCpoLineoff = 'o',
  kCpoListwm = 'L',
  kCpoLiteral = 'l',
  kCpoMatch = '%',
  kCpoMatchbsl = 'M',
  kCpoNosymlinks = '~',
  kCpoNumcol = 'n',
  kCpoOvernew = 'O',
  kCpoPlus = '+',
  kCpoRedo = 'r',
  kCpoRegappend = '>',
  kCpoRemmark = 'R',
  kCpoReplcnt = 'X',
  kCpoScolon = ';',
  kCpoSearch = 'c',
  kCpoShowmatch = 'm',
  kCpoTagpat = 't',
  kCpoUndo = 'u',
  kCpoYank = 'y',
};

typedef enum {
  kOptCuloptFlagLine = 0x01,
  kOptCuloptFlagScreenline = 0x02,
  kOptCuloptFlagNumber = 0x04,
} OptCuloptFlags;

typedef enum {
  kOptDyFlagLastline = 0x01,
  kOptDyFlagTruncate = 0x02,
  kOptDyFlagUhex = 0x04,
  kOptDyFlagMsgsep = 0x08,
} OptDyFlags;

typedef enum {
  kOptFdoFlagAll = 0x01,
  kOptFdoFlagBlock = 0x02,
  kOptFdoFlagHor = 0x04,
  kOptFdoFlagMark = 0x08,
  kOptFdoFlagPercent = 0x10,
  kOptFdoFlagQuickfix = 0x20,
  kOptFdoFlagSearch = 0x40,
  kOptFdoFlagTag = 0x80,
  kOptFdoFlagInsert = 0x100,
  kOptFdoFlagUndo = 0x200,
  kOptFdoFlagJump = 0x400,
} OptFdoFlags;

enum {
  kFoAuto = 'a',
  kFoInsBlank = 'b',
  kFoInsLong = 'l',
  kFoInsVi = 'v',
  kFoMbyteBreak = 'm',
  kFoMbyteJoin = 'M',
  kFoMbyteJoin2 = 'B',
  kFoNoOpenComs = '/',
  kFoOneLetter = '1',
  kFoOpenComs = 'o',
  kFoPeriodAbbr = 'p',
  kFoQComs = 'q',
  kFoQNumber = 'n',
  kFoQSecond = '2',
  kFoRemoveComs = 'j',
  kFoRetComs = 'r',
  kFoRigorousTw = ']',
  kFoWhitePar = 'w',
  kFoWrap = 't',
  kFoWrapComs = 'c',
};

typedef enum {
  kOptJopFlagStack = 0x01,
  kOptJopFlagView = 0x02,
  kOptJopFlagClean = 0x04,
} OptJopFlags;

enum {
  kMouseCommand = 'c',
  kMouseHelp = 'h',
  kMouseInsert = 'i',
  kMouseNone = ' ',
  kMouseNonef = 'x',
  kMouseNormal = 'n',
  kMouseReturn = 'r',
  kMouseVisual = 'v',
};

typedef enum {
  kOptRdbFlagCompositor = 0x01,
  kOptRdbFlagNothrottle = 0x02,
  kOptRdbFlagInvalid = 0x04,
  kOptRdbFlagNodelta = 0x08,
  kOptRdbFlagLine = 0x10,
  kOptRdbFlagFlush = 0x20,
} OptRdbFlags;

typedef enum {
  kOptSsopFlagBuffers = 0x01,
  kOptSsopFlagWinpos = 0x02,
  kOptSsopFlagResize = 0x04,
  kOptSsopFlagWinsize = 0x08,
  kOptSsopFlagLocaloptions = 0x10,
  kOptSsopFlagOptions = 0x20,
  kOptSsopFlagHelp = 0x40,
  kOptSsopFlagBlank = 0x80,
  kOptSsopFlagGlobals = 0x100,
  kOptSsopFlagSlash = 0x200,
  kOptSsopFlagUnix = 0x400,
  kOptSsopFlagSesdir = 0x800,
  kOptSsopFlagCurdir = 0x1000,
  kOptSsopFlagFolds = 0x2000,
  kOptSsopFlagCursor = 0x4000,
  kOptSsopFlagTabpages = 0x8000,
  kOptSsopFlagTerminal = 0x10000,
  kOptSsopFlagSkiprtp = 0x20000,
} OptSsopFlags;

enum {
  kShmAbbreviations = 'a',
  kShmAttention = 'A',
  kShmCompletionmenu = 'c',
  kShmCompletionscan = 'C',
  kShmFileinfo = 'F',
  kShmIntro = 'I',
  kShmLines = 'l',
  kShmMod = 'm',
  kShmOver = 'o',
  kShmOverall = 'O',
  kShmRecording = 'q',
  kShmRo = 'r',
  kShmSearch = 's',
  kShmSearchcount = 'S',
  kShmTrunc = 't',
  kShmTruncall = 'T',
  kShmUndo = 'u',
  kShmWri = 'w',
  kShmWrite = 'W',
};

typedef enum {
  kOptSpoFlagCamel = 0x01,
  kOptSpoFlagNoplainbuffer = 0x02,
} OptSpoFlags;

typedef enum {
  kOptSwbFlagUseopen = 0x01,
  kOptSwbFlagUsetab = 0x02,
  kOptSwbFlagSplit = 0x04,
  kOptSwbFlagNewtab = 0x08,
  kOptSwbFlagVsplit = 0x10,
  kOptSwbFlagUselast = 0x20,
} OptSwbFlags;

typedef enum {
  kOptTclFlagLeft = 0x01,
  kOptTclFlagUselast = 0x02,
} OptTclFlags;

typedef enum {
  kOptTcFlagFollowic = 0x01,
  kOptTcFlagIgnore = 0x02,
  kOptTcFlagMatch = 0x04,
  kOptTcFlagFollowscs = 0x08,
  kOptTcFlagSmart = 0x10,
} OptTcFlags;

typedef enum {
  kOptTpfFlagBS = 0x01,
  kOptTpfFlagHT = 0x02,
  kOptTpfFlagFF = 0x04,
  kOptTpfFlagESC = 0x08,
  kOptTpfFlagDEL = 0x10,
  kOptTpfFlagC0 = 0x20,
  kOptTpfFlagC1 = 0x40,
} OptTpfFlags;

typedef enum {
  kOptVeFlagAll = 0x04,
  kOptVeFlagBlock = 0x05,
  kOptVeFlagInsert = 0x06,
  kOptVeFlagOnemore = 0x08,
  kOptVeFlagNone = 0x10,
  kOptVeFlagNoneU = 0x20,
} OptVeFlags;

typedef enum {
  kOptWimFlagFull = 0x01,
  kOptWimFlagLongest = 0x02,
  kOptWimFlagList = 0x04,
  kOptWimFlagLastused = 0x08,
  kOptWimFlagNoselect = 0x10,
  kOptWimFlagNoinsert = 0x20,
} OptWimFlags;

typedef enum {
  kOptWopFlagFuzzy = 0x01,
  kOptWopFlagTagfile = 0x02,
  kOptWopFlagPum = 0x04,
  kOptWopFlagExacttext = 0x08,
} OptWopFlags;

EXTERN const char *(opt_ambw_values[3])INIT( = {
  "single",
  "double",
  NULL
});

EXTERN const char *(opt_bg_values[3])INIT( = {
  "light",
  "dark",
  NULL
});

EXTERN const char *(opt_bs_values[5])INIT( = {
  "indent",
  "eol",
  "start",
  "nostop",
  NULL
});

EXTERN const char *(opt_bkc_values[6])INIT( = {
  "yes",
  "auto",
  "no",
  "breaksymlink",
  "breakhardlink",
  NULL
});

EXTERN const char *(opt_bo_values[21])INIT( = {
  "all",
  "backspace",
  "cursor",
  "complete",
  "copy",
  "ctrlg",
  "error",
  "esc",
  "ex",
  "hangul",
  "insertmode",
  "lang",
  "mess",
  "showmatch",
  "operator",
  "register",
  "shell",
  "spell",
  "term",
  "wildmode",
  NULL
});

EXTERN const char *(opt_briopt_values[6])INIT( = {
  "shift:",
  "min:",
  "sbr",
  "list:",
  "column:",
  NULL
});

EXTERN const OptSchemaItem opt_briopt_schema[] INIT( = {
  { "shift", kOptSchemaSNum, NULL },
  { "min", kOptSchemaNum, NULL },
  { "sbr", kOptSchemaFlag, NULL },
  { "list", kOptSchemaSNum, NULL },
  { "column", kOptSchemaSNum, NULL },
  { NULL, kOptSchemaFlag, NULL },
});

EXTERN const char *(opt_bh_values[6])INIT( = {
  "",
  "hide",
  "unload",
  "delete",
  "wipe",
  NULL
});

EXTERN const char *(opt_bt_values[9])INIT( = {
  "",
  "acwrite",
  "help",
  "nofile",
  "nowrite",
  "quickfix",
  "terminal",
  "prompt",
  NULL
});

EXTERN const char *(opt_cmp_values[3])INIT( = {
  "internal",
  "keepascii",
  NULL
});

EXTERN const char *(opt_cb_values[3])INIT( = {
  "unnamed",
  "unnamedplus",
  NULL
});

EXTERN const char *(opt_cpt_values[16])INIT( = {
  ".",
  "w",
  "b",
  "u",
  "k",
  "kspell",
  "s",
  "i",
  "d",
  "]",
  "t",
  "U",
  "f",
  "F",
  "o",
  NULL
});

EXTERN const char *(opt_cot_values[13])INIT( = {
  "fuzzy",
  "longest",
  "menu",
  "menuone",
  "nearest",
  "noinsert",
  "noselect",
  "nosort",
  "popup",
  "preinsert",
  "preselect",
  "preview",
  NULL
});

EXTERN const char *(opt_csl_values[4])INIT( = {
  "",
  "slash",
  "backslash",
  NULL
});

EXTERN const char *(opt_culopt_values[5])INIT( = {
  "line",
  "screenline",
  "number",
  "both",
  NULL
});

EXTERN const char *(opt_debug_values[4])INIT( = {
  "msg",
  "throw",
  "beep",
  NULL
});

EXTERN const char *(opt_dip_values[20])INIT( = {
  "filler",
  "anchor",
  "context:",
  "iblank",
  "icase",
  "iwhite",
  "iwhiteall",
  "iwhiteeol",
  "horizontal",
  "vertical",
  "closeoff",
  "hiddenoff",
  "foldcolumn:",
  "followwrap",
  "internal",
  "indent-heuristic",
  "algorithm:",
  "inline:",
  "linematch:",
  NULL
});

EXTERN const char *(opt_dip_algorithm_values[5])INIT( = {
  "myers",
  "minimal",
  "patience",
  "histogram",
  NULL
});

EXTERN const char *(opt_dip_inline_values[5])INIT( = {
  "none",
  "simple",
  "char",
  "word",
  NULL
});

EXTERN const OptSchemaItem opt_dip_schema[] INIT( = {
  { "filler", kOptSchemaFlag, NULL },
  { "anchor", kOptSchemaFlag, NULL },
  { "context", kOptSchemaNum, NULL },
  { "iblank", kOptSchemaFlag, NULL },
  { "icase", kOptSchemaFlag, NULL },
  { "iwhite", kOptSchemaFlag, NULL },
  { "iwhiteall", kOptSchemaFlag, NULL },
  { "iwhiteeol", kOptSchemaFlag, NULL },
  { "horizontal", kOptSchemaFlag, NULL },
  { "vertical", kOptSchemaFlag, NULL },
  { "closeoff", kOptSchemaFlag, NULL },
  { "hiddenoff", kOptSchemaFlag, NULL },
  { "foldcolumn", kOptSchemaNum, NULL },
  { "followwrap", kOptSchemaFlag, NULL },
  { "internal", kOptSchemaFlag, NULL },
  { "indent-heuristic", kOptSchemaFlag, NULL },
  { "algorithm", kOptSchemaEnum, opt_dip_algorithm_values },
  { "inline", kOptSchemaEnum, opt_dip_inline_values },
  { "linematch", kOptSchemaNum, NULL },
  { NULL, kOptSchemaFlag, NULL },
});

EXTERN const char *(opt_dy_values[5])INIT( = {
  "lastline",
  "truncate",
  "uhex",
  "msgsep",
  NULL
});

EXTERN const char *(opt_ead_values[4])INIT( = {
  "both",
  "ver",
  "hor",
  NULL
});

EXTERN const char *(opt_ff_values[4])INIT( = {
  "unix",
  "dos",
  "mac",
  NULL
});

EXTERN const char *(opt_fcl_values[2])INIT( = {
  "all",
  NULL
});

EXTERN const char *(opt_fdc_values[21])INIT( = {
  "auto",
  "auto:1",
  "auto:2",
  "auto:3",
  "auto:4",
  "auto:5",
  "auto:6",
  "auto:7",
  "auto:8",
  "auto:9",
  "0",
  "1",
  "2",
  "3",
  "4",
  "5",
  "6",
  "7",
  "8",
  "9",
  NULL
});

EXTERN const char *(opt_fdm_values[7])INIT( = {
  "manual",
  "expr",
  "marker",
  "indent",
  "syntax",
  "diff",
  NULL
});

EXTERN const char *(opt_fdo_values[12])INIT( = {
  "all",
  "block",
  "hor",
  "mark",
  "percent",
  "quickfix",
  "search",
  "tag",
  "insert",
  "undo",
  "jump",
  NULL
});

EXTERN const char *(opt_icm_values[4])INIT( = {
  "nosplit",
  "split",
  "",
  NULL
});

EXTERN const char *(opt_jop_values[4])INIT( = {
  "stack",
  "view",
  "clean",
  NULL
});

EXTERN const char *(opt_km_values[3])INIT( = {
  "startsel",
  "stopsel",
  NULL
});

EXTERN const char *(opt_lop_values[3])INIT( = {
  "expr:0",
  "expr:1",
  NULL
});

EXTERN const char *(opt_mopt_values[8])INIT( = {
  "hit-enter",
  "history:",
  "maxheight:",
  "pager:",
  "progress:",
  "timeout:",
  "wait:",
  NULL
});

EXTERN const char *(opt_mopt_progress_values[3])INIT( = {
  "",
  "c",
  NULL
});

EXTERN const OptSchemaItem opt_mopt_schema[] INIT( = {
  { "hit-enter", kOptSchemaFlag, NULL },
  { "history", kOptSchemaNum, NULL },
  { "maxheight", kOptSchemaNum, NULL },
  { "pager", kOptSchemaStr, NULL },
  { "progress", kOptSchemaEnum, opt_mopt_progress_values },
  { "timeout", kOptSchemaNum, NULL },
  { "wait", kOptSchemaNum, NULL },
  { NULL, kOptSchemaFlag, NULL },
});

EXTERN const char *(opt_mousem_values[4])INIT( = {
  "extend",
  "popup",
  "popup_setpos",
  NULL
});

EXTERN const char *(opt_mousescroll_values[3])INIT( = {
  "hor:",
  "ver:",
  NULL
});

EXTERN const OptSchemaItem opt_mousescroll_schema[] INIT( = {
  { "hor", kOptSchemaNum, NULL },
  { "ver", kOptSchemaNum, NULL },
  { NULL, kOptSchemaFlag, NULL },
});

EXTERN const char *(opt_nf_values[7])INIT( = {
  "bin",
  "octal",
  "hex",
  "alpha",
  "unsigned",
  "blank",
  NULL
});

EXTERN const char *(opt_pvp_values[4])INIT( = {
  "height:",
  "width:",
  "border:",
  NULL
});

EXTERN const char *(opt_pvp_border_values[8])INIT( = {
  "double",
  "single",
  "shadow",
  "rounded",
  "solid",
  "bold",
  "none",
  NULL
});

EXTERN const OptSchemaItem opt_pvp_schema[] INIT( = {
  { "height", kOptSchemaNum, NULL },
  { "width", kOptSchemaNum, NULL },
  { "border", kOptSchemaEnum, opt_pvp_border_values },
  { NULL, kOptSchemaFlag, NULL },
});

EXTERN const char *(opt_pumborder_values[9])INIT( = {
  "",
  "double",
  "single",
  "shadow",
  "rounded",
  "solid",
  "bold",
  "none",
  NULL
});

EXTERN const char *(opt_rdb_values[7])INIT( = {
  "compositor",
  "nothrottle",
  "invalid",
  "nodelta",
  "line",
  "flush",
  NULL
});

EXTERN const char *(opt_rlc_values[2])INIT( = {
  "search",
  NULL
});

EXTERN const char *(opt_sbo_values[4])INIT( = {
  "ver",
  "hor",
  "jump",
  NULL
});

EXTERN const char *(opt_sel_values[4])INIT( = {
  "inclusive",
  "exclusive",
  "old",
  NULL
});

EXTERN const char *(opt_slm_values[4])INIT( = {
  "mouse",
  "key",
  "cmd",
  NULL
});

EXTERN const char *(opt_ssop_values[19])INIT( = {
  "buffers",
  "winpos",
  "resize",
  "winsize",
  "localoptions",
  "options",
  "help",
  "blank",
  "globals",
  "slash",
  "unix",
  "sesdir",
  "curdir",
  "folds",
  "cursor",
  "tabpages",
  "terminal",
  "skiprtp",
  NULL
});

EXTERN const char *(opt_sloc_values[4])INIT( = {
  "last",
  "statusline",
  "tabline",
  NULL
});

EXTERN const char *(opt_scl_values[23])INIT( = {
  "yes",
  "no",
  "auto",
  "auto:1",
  "auto:2",
  "auto:3",
  "auto:4",
  "auto:5",
  "auto:6",
  "auto:7",
  "auto:8",
  "auto:9",
  "yes:1",
  "yes:2",
  "yes:3",
  "yes:4",
  "yes:5",
  "yes:6",
  "yes:7",
  "yes:8",
  "yes:9",
  "number",
  NULL
});

EXTERN const char *(opt_spo_values[3])INIT( = {
  "camel",
  "noplainbuffer",
  NULL
});

EXTERN const char *(opt_sps_values[7])INIT( = {
  "best",
  "fast",
  "double",
  "expr:",
  "file:",
  "timeout:",
  NULL
});

EXTERN const char *(opt_spk_values[4])INIT( = {
  "cursor",
  "screen",
  "topline",
  NULL
});

EXTERN const char *(opt_swb_values[7])INIT( = {
  "useopen",
  "usetab",
  "split",
  "newtab",
  "vsplit",
  "uselast",
  NULL
});

EXTERN const char *(opt_tcl_values[3])INIT( = {
  "left",
  "uselast",
  NULL
});

EXTERN const char *(opt_tc_values[6])INIT( = {
  "followic",
  "ignore",
  "match",
  "followscs",
  "smart",
  NULL
});

EXTERN const char *(opt_tpf_values[8])INIT( = {
  "BS",
  "HT",
  "FF",
  "ESC",
  "DEL",
  "C0",
  "C1",
  NULL
});

EXTERN const char *(opt_ve_values[7])INIT( = {
  "block",
  "insert",
  "all",
  "onemore",
  "none",
  "NONE",
  NULL
});

EXTERN const char *(opt_wim_values[7])INIT( = {
  "full",
  "longest",
  "list",
  "lastused",
  "noselect",
  "noinsert",
  NULL
});

EXTERN const char *(opt_wop_values[5])INIT( = {
  "fuzzy",
  "tagfile",
  "pum",
  "exacttext",
  NULL
});

EXTERN const char *(opt_wak_values[4])INIT( = {
  "yes",
  "menu",
  "no",
  NULL
});

EXTERN const char *(opt_winborder_values[9])INIT( = {
  "",
  "double",
  "single",
  "shadow",
  "rounded",
  "solid",
  "bold",
  "none",
  NULL
});

// option_vars.h: definition of global variables for settable options

#define HIGHLIGHT_INIT \
  "8:SpecialKey,~:EndOfBuffer,z:TermCursor,@:NonText,d:Directory,e:ErrorMsg,i:IncSearch,l:Search," \
  "y:CurSearch,m:MoreMsg,M:ModeMsg,n:LineNr,a:LineNrAbove,b:LineNrBelow,N:CursorLineNr," \
  "G:CursorLineSign,O:CursorLineFold,r:Question,s:StatusLine,S:StatusLineNC,c:VertSplit,t:Title," \
  "v:Visual,V:VisualNOS,w:WarningMsg,W:WildMenu,f:Folded,F:FoldColumn,A:DiffAdd,C:DiffChange," \
  "D:DiffDelete,T:DiffText,E:DiffTextAdd,>:SignColumn,-:Conceal,B:SpellBad,P:SpellCap," \
  "R:SpellRare,L:SpellLocal,+:Pmenu,=:PmenuSel,k:PmenuMatch,<:PmenuMatchSel,[:PmenuKind," \
  "]:PmenuKindSel,{:PmenuExtra,}:PmenuExtraSel,x:PmenuSbar,X:PmenuThumb,*:TabLine,#:TabLineSel," \
  "_:TabLineFill,!:CursorColumn,.:CursorLine,o:ColorColumn,q:QuickFixLine,z:StatusLineTerm," \
  "Z:StatusLineTermNC,g:MsgArea,h:ComplMatchIns,0:Whitespace,I:PreInsert"

// Default values for 'errorformat'.
// The "%f|%l| %m" one is used for when the contents of the quickfix window is
// written to a file.
#ifdef MSWIN
# define DFLT_EFM \
  "%f(%l): %t%*\\D%n: %m,%f(%l\\,%c): %t%*\\D%n: %m,%f(%l) \\=: %t%*\\D%n: %m,%*[^\"]\"%f\"%*\\D%l: %m,%f(%l) \\=: %m,%*[^ ] %f %l: %m,%f:%l:%c:%m,%f(%l):%m,%f:%l:%m,%f|%l| %m"
#else
# define DFLT_EFM \
  "%*[^\"]\"%f\"%*\\D%l: %m,\"%f\"%*\\D%l: %m,%-Gg%\\?make[%*\\d]: *** [%f:%l:%m,%-Gg%\\?make: *** [%f:%l:%m,%-G%f:%l: (Each undeclared identifier is reported only once,%-G%f:%l: for each function it appears in.),%-GIn file included from %f:%l:%c:,%-GIn file included from %f:%l:%c\\,,%-GIn file included from %f:%l:%c,%-GIn file included from %f:%l,%-G%*[ ]from %f:%l:%c,%-G%*[ ]from %f:%l:,%-G%*[ ]from %f:%l\\,,%-G%*[ ]from %f:%l,%f:%l:%c:%m,%f(%l):%m,%f:%l:%m,\"%f\"\\, line %l%*\\D%c%*[^ ] %m,%D%*\\a[%*\\d]: Entering directory %*[`']%f',%X%*\\a[%*\\d]: Leaving directory %*[`']%f',%D%*\\a: Entering directory %*[`']%f',%X%*\\a: Leaving directory %*[`']%f',%DMaking %*\\a in %f,%f|%l| %m"
#endif

// Default values for 'guifont'
#ifdef MSWIN
# define DFLT_GFN "Cascadia Code,Cascadia Mono,Consolas,Courier New,monospace"
#elif defined(__APPLE__)
# define DFLT_GFN "SF Mono,Menlo,Monaco,Courier New,monospace"
#elif defined(__linux__)
# define DFLT_GFN "Source Code Pro,DejaVu Sans Mono,Courier New,monospace"
#else
# define DFLT_GFN "DejaVu Sans Mono,Courier New,monospace"
#endif

#define DFLT_GREPFORMAT "%f:%l:%m,%f:%l%m,%f  %l%m"

// Possible values for 'encoding'
#define ENC_UCSBOM     "ucs-bom"       // check for BOM at start of file

// default value for 'encoding'
#define ENC_DFLT       "utf-8"

// end-of-line style
#define EOL_UNKNOWN     (-1)    // not defined yet
#define EOL_UNIX        0       // NL
#define EOL_DOS         1       // CR NL
#define EOL_MAC         2       // CR

// Formatting options for 'formatoptions': the per-flag kFo* constants are generated from the
// option's `flagchars` schema (options.lua); FO_ALL is the concatenated set for do_set().
#define DFLT_FO_VI      "vt"
#define DFLT_FO_VIM     "tcqj"
#define FO_ALL          "tcro/q2vlb1mMBn,aw]jp"   // for do_set()

#define MAX_MCO  6  // fixed value for 'maxcombine'

// characters for the p_cpo option: the per-flag kCpo* constants are generated from the
// option's `flagchars` schema (options.lua). CPO_VI/CPO_VIM are the Vi/Vim default sets.
// default values for Vim and Vi
#define CPO_VIM         "aABceFs_"
#define CPO_VI          "aAbBcCdDeEfFiIJKlLmMnoOpPqrRsStuvWxXyZ$!%+>;~_"

// characters for p_ww option:
#define WW_ALL          "bshl<>[]~"

// characters for p_mouse option: the per-flag kMouse* constants are generated from the option's
// `flagchars` schema (options.lua).
#define MOUSE_A         "nvich"         // used for 'a' flag
#define MOUSE_ALL       "anvichr"       // all possible characters

// default vertical and horizontal mouse scroll values.
// Note: This should be in sync with the default mousescroll option.
#define MOUSESCROLL_VERT_DFLT   3
#define MOUSESCROLL_HOR_DFLT    6

#define COCU_ALL        "nvic"          // flags for 'concealcursor'

// characters for the p_shm option: the per-flag kShm* constants are generated from the option's
// `flagchars` schema (options.lua).
/// Represented by 'a' flag.
#define SHM_ALL_ABBREVIATIONS ((char[]) { \
    kShmRo, kShmMod, kShmLines, kShmWri, \
    0 })

// characters for p_go:
#define GO_ASEL         'a'             // autoselect
#define GO_ASELML       'A'             // autoselect modeless selection
#define GO_BOT          'b'             // use bottom scrollbar
#define GO_CONDIALOG    'c'             // use console dialog
#define GO_TITLEBAR     'C'             // use 'hl-TitleBar'
#define GO_DARKTHEME    'd'             // use dark theme variant
#define GO_TABLINE      'e'             // may show tabline
#define GO_FORG         'f'             // start GUI in foreground
#define GO_GREY         'g'             // use grey menu items
#define GO_HORSCROLL    'h'             // flexible horizontal scrolling
#define GO_ICON         'i'             // use Vim icon
#define GO_LEFT         'l'             // use left scrollbar
#define GO_VLEFT        'L'             // left scrollbar with vert split
#define GO_MENUS        'm'             // use menu bar
#define GO_NOSYSMENU    'M'             // don't source system menu
#define GO_POINTER      'p'             // pointer enter/leave callbacks
#define GO_ASELPLUS     'P'             // autoselectPlus
#define GO_RIGHT        'r'             // use right scrollbar
#define GO_VRIGHT       'R'             // right scrollbar with vert split
#define GO_TOOLBAR      'T'             // add toolbar
#define GO_FOOTER       'F'             // add footer
#define GO_VERTICAL     'v'             // arrange dialog buttons vertically
#define GO_KEEPWINSIZE  'k'             // keep GUI window size
#define GO_ALL "!aAbcCdefFghilLmMpPrRtTvk"  // all possible flags for 'go'

// flags for 'comments' option
#define COM_NEST        'n'             // comments strings nest
#define COM_BLANK       'b'             // needs blank after string
#define COM_START       's'             // start of comment
#define COM_MIDDLE      'm'             // middle of comment
#define COM_END         'e'             // end of comment
#define COM_AUTO_END    'x'             // last char of end closes comment
#define COM_FIRST       'f'             // first line comment only
#define COM_LEFT        'l'             // left adjusted
#define COM_RIGHT       'r'             // right adjusted
#define COM_NOBACK      'O'             // don't use for "O" command
#define COM_ALL         "nbsmexflrO"    // all flags for 'comments' option
#define COM_MAX_LEN     50              // maximum length of a part

/// C string containing all 'statusline' option flags
#define STL_ALL ((char[]) { \
    STL_FILEPATH, STL_FULLPATH, STL_FILENAME, STL_COLUMN, STL_VIRTCOL, \
    STL_VIRTCOL_ALT, STL_LINE, STL_NUMLINES, STL_BUFNO, STL_KEYMAP, STL_OFFSET, \
    STL_OFFSET_X, STL_BYTEVAL, STL_BYTEVAL_X, STL_ROFLAG, STL_ROFLAG_ALT, \
    STL_HELPFLAG, STL_HELPFLAG_ALT, STL_FILETYPE, STL_FILETYPE_ALT, \
    STL_PREVIEWFLAG, STL_PREVIEWFLAG_ALT, STL_MODIFIED, STL_MODIFIED_ALT, \
    STL_QUICKFIX, STL_PERCENTAGE, STL_ALTPERCENT, STL_ARGLISTSTAT, STL_PAGENUM, \
    STL_SHOWCMD, STL_FOLDCOL, STL_SIGNCOL, STL_VIM_EXPR, STL_SEPARATE, \
    STL_TRUNCMARK, STL_USER_HL, STL_HIGHLIGHT, STL_HIGHLIGHT_COMB, STL_TABPAGENR, \
    STL_TABCLOSENR, STL_CLICK_FUNC, STL_TABPAGENR, STL_TABCLOSENR, STL_CLICK_FUNC, \
    0, })

// arguments for can_bs()
// each defined char should be unique over all values
// except for BS_START, that intentionally also matches BS_NOSTOP
// because BS_NOSTOP behaves exactly the same except it
// does not stop at the start of the insert point
#define BS_INDENT       'i'     // "Indent"
#define BS_EOL          'l'     // "eoL"
#define BS_START        's'     // "Start"
#define BS_NOSTOP       'p'     // "nostoP

#define LISPWORD_VALUE \
  "defun,define,defmacro,set!,lambda,if,case,let,flet,let*,letrec,do,do*,define-syntax,let-syntax,letrec-syntax,destructuring-bind,defpackage,defparameter,defstruct,deftype,defvar,do-all-symbols,do-external-symbols,do-symbols,dolist,dotimes,ecase,etypecase,eval-when,labels,macrolet,multiple-value-bind,multiple-value-call,multiple-value-prog1,multiple-value-setq,prog1,progv,typecase,unless,unwind-protect,when,with-input-from-string,with-open-file,with-open-stream,with-output-to-string,with-package-iterator,define-condition,handler-bind,handler-case,restart-bind,restart-case,with-simple-restart,store-value,use-value,muffle-warning,abort,continue,with-slots,with-slots*,with-accessors,with-accessors*,defclass,defmethod,print-unreadable-object"

// When a string option is NULL, it is set to empty_string_option,
// to avoid having to check for NULL everywhere.
//
// TODO(famiu): Remove this when refcounted strings are used for string options.
extern char empty_string_option[];

// The following are actual variables for the options

extern char *p_ambw;             ///< 'ambiwidth'
extern int p_acd;                ///< 'autochdir'
extern int p_ai;                 ///< 'autoindent'
extern int p_bin;                ///< 'binary'
extern int p_bomb;               ///< 'bomb'
extern int p_bl;                 ///< 'buflisted'
extern int p_cin;                ///< 'cindent'
extern OptInt p_channel;         ///< 'channel'
extern char *p_cink;             ///< 'cinkeys'
extern char *p_cinsd;            ///< 'cinscopedecls'
extern char *p_cinw;             ///< 'cinwords'
extern Callback p_cfu;          ///< 'completefunc'
extern Callback p_ofu;          ///< 'omnifunc'
extern Callback p_tsrfu;        ///< 'thesaurusfunc'
extern int p_ci;                 ///< 'copyindent'
extern int p_ar;                 ///< 'autoread'
extern int p_aw;                 ///< 'autowrite'
extern int p_awa;                ///< 'autowriteall'
extern char *p_bs;               ///< 'backspace'
extern char *p_bg;               ///< 'background'
extern int p_bk;                 ///< 'backup'
extern char *p_bkc;              ///< 'backupcopy'
extern unsigned bkc_flags;       ///< flags from 'backupcopy'
extern char *p_bdir;             ///< 'backupdir'
extern char *p_bex;              ///< 'backupext'
extern char *p_bo;               ///< 'belloff'
extern char breakat_flags[256];  ///< which characters are in 'breakat'
extern unsigned bo_flags;
extern char *p_bsk;             ///< 'backupskip'
extern char *p_breakat;         ///< 'breakat'
extern char *p_bh;              ///< 'bufhidden'
extern char *p_bt;              ///< 'buftype'
extern OptInt p_busy;           ///< 'busy'
extern char *p_cmp;             ///< 'casemap'
extern unsigned cmp_flags;
extern char *p_enc;             ///< 'encoding'
extern int p_deco;              ///< 'delcombine'
extern Callback p_ccv;          ///< 'charconvert'
extern char *p_cino;            ///< 'cinoptions'
extern char *p_cedit;           ///< 'cedit'
extern char *p_cb;              ///< 'clipboard'
extern unsigned cb_flags;
extern OptInt p_cwh;            ///< 'cmdwinheight'
extern OptInt p_ch;             ///< 'cmdheight'
extern char *p_cms;             ///< 'commentstring'
extern char *p_cpt;             ///< 'complete'
extern OptInt p_cto;            ///< 'completetimeout'
extern OptInt p_columns;        ///< 'columns'
extern int p_confirm;           ///< 'confirm'
extern char *p_cia;             ///< 'completeitemalign'
extern unsigned cia_flags;      ///< order flags of 'completeitemalign'
extern char *p_cot;             ///< 'completeopt'
extern unsigned cot_flags;      ///< flags from 'completeopt'
extern int p_ac;                ///< 'autocomplete'
extern OptInt p_act;            ///< 'autocompletetimeout'
extern OptInt p_acl;            ///< 'autocompletedelay'
#ifdef BACKSLASH_IN_FILENAME
extern char *p_csl;             ///< 'completeslash'
#endif
extern char *p_pumborder;       ///< 'pumborder'
extern OptInt p_pb;             ///< 'pumblend'
extern OptInt p_ph;             ///< 'pumheight'
extern OptInt p_pw;             ///< 'pumwidth'
extern OptInt p_pmw;            ///< 'pummaxwidth'
extern char *p_com;             ///< 'comments'
extern char *p_cpo;             ///< 'cpoptions'
extern char *p_debug;           ///< 'debug'
extern char *p_def;             ///< 'define'
extern char *p_inc;
extern char *p_dia;             ///< 'diffanchors'
extern char *p_dip;             ///< 'diffopt'
extern Callback p_dex;          ///< 'diffexpr'
extern char *p_dict;            ///< 'dictionary'
extern int p_dg;                ///< 'digraph'
extern char *p_dir;             ///< 'directory'
extern char *p_dy;              ///< 'display'
extern unsigned dy_flags;
extern int dy_escape_width;
extern char *p_ead;             ///< 'eadirection'
extern int p_emoji;             ///< 'emoji'
extern int p_ea;                ///< 'equalalways'
extern char *p_ep;              ///< 'equalprg'
extern int p_eb;                ///< 'errorbells'
extern char *p_ef;              ///< 'errorfile'
extern char *p_efm;             ///< 'errorformat'
extern char *p_gefm;            ///< 'grepformat'
extern char *p_gp;              ///< 'grepprg'
extern int p_eof;               ///< 'endoffile'
extern int p_eol;               ///< 'endofline'
extern char *p_ei;              ///< 'eventignore'
extern int p_et;                ///< 'expandtab'
extern int p_exrc;              ///< 'exrc'
extern char *p_fenc;            ///< 'fileencoding'
extern char *p_fencs;           ///< 'fileencodings'
extern char *p_ff;              ///< 'fileformat'
extern char *p_ffs;             ///< 'fileformats'
extern int p_fic;               ///< 'fileignorecase'
extern char *p_ft;              ///< 'filetype'
extern char *p_fcs;             ///< 'fillchar'
extern Callback p_ffu;          ///< 'findfunc'
extern int p_fixeol;            ///< 'fixendofline'
extern char *p_fcl;             ///< 'foldclose'
extern OptInt p_fdls;           ///< 'foldlevelstart'
extern char *p_fdo;             ///< 'foldopen'
extern unsigned fdo_flags;
extern int p_follow;            ///< 'follow'
extern Callback p_fex;          ///< 'formatexpr'
extern char *p_flp;             ///< 'formatlistpat'
extern char *p_fo;              ///< 'formatoptions'
extern char *p_fp;              ///< 'formatprg'
extern int p_fs;                ///< 'fsync'
extern int p_gd;                ///< 'gdefault'
extern char *p_guicursor;       ///< 'guicursor'
extern char *p_guifont;         ///< 'guifont'
extern char *p_guifontwide;     ///< 'guifontwide'
extern char *p_hf;              ///< 'helpfile'
extern OptInt p_hh;             ///< 'helpheight'
extern char *p_hlg;             ///< 'helplang'
extern int p_hid;               ///< 'hidden'
extern char *p_hl;              ///< 'highlight'
extern int p_hls;               ///< 'hlsearch'
extern OptInt p_hi;             ///< 'history'
extern int p_arshape;           ///< 'arabicshape'
extern int p_icon;              ///< 'icon'
extern char *p_iconstring;      ///< 'iconstring'
extern int p_ic;                ///< 'ignorecase'
extern OptInt p_iminsert;       ///< 'iminsert'
extern OptInt p_imsearch;       ///< 'imsearch'
extern int p_inf;               ///< 'infercase'
extern Callback p_inex;         ///< 'includeexpr'
extern int p_is;                ///< 'incsearch'
extern Callback p_inde;         ///< 'indentexpr'
extern char *p_indk;            ///< 'indentkeys'
extern char *p_icm;             ///< 'inccommand'
extern char *p_isf;             ///< 'isfname'
extern char *p_isi;             ///< 'isident'
extern char *p_isk;             ///< 'iskeyword'
extern int p_js;                ///< 'joinspaces'
extern char *p_jop;             ///< 'jumpooptions'
extern unsigned jop_flags;
extern char *p_keymap;          ///< 'keymap'
extern char *p_kp;              ///< 'keywordprg'
extern char *p_km;              ///< 'keymodel'
extern char *p_langmap;         ///< 'langmap'
extern int p_lnr;               ///< 'langnoremap'
extern int p_lrm;               ///< 'langremap'
extern char *p_lm;              ///< 'langmenu'
extern OptInt p_lines;          ///< 'lines'
extern OptInt p_linespace;      ///< 'linespace'
extern int p_lisp;              ///< 'lisp'
extern char *p_lop;             ///< 'lispoptions'
extern char *p_lispwords;       ///< 'lispwords'
extern OptInt p_ls;             ///< 'laststatus'
extern OptInt p_stal;           ///< 'showtabline'
extern char *p_lcs;             ///< 'listchars'
extern int p_lz;                ///< 'lazyredraw'
extern int p_lpl;               ///< 'loadplugins'
extern int p_magic;             ///< 'magic'
extern char *p_menc;            ///< 'makeencoding'
extern char *p_mef;             ///< 'makeef'
extern char *p_mp;              ///< 'makeprg'
extern char *p_mps;             ///< 'matchpairs'
extern OptInt p_mat;            ///< 'matchtime'
extern OptInt p_mco;            ///< 'maxcombine'
extern OptInt p_mfd;            ///< 'maxfuncdepth'
extern OptInt p_mmd;            ///< 'maxmapdepth'
extern OptInt p_mmp;            ///< 'maxmempattern'
extern OptInt p_mis;            ///< 'menuitems'
extern char *p_mopt;            ///< 'messagesopt'
extern OptInt p_msc;            ///< 'maxsearchcount'
extern char *p_msm;             ///< 'mkspellmem'
extern int p_ml;                ///< 'modeline'
extern int p_mle;               ///< 'modelineexpr'
extern OptInt p_mls;            ///< 'modelines'
extern int p_ma;                ///< 'modifiable'
extern int p_mod;               ///< 'modified'
extern char *p_mouse;           ///< 'mouse'
extern char *p_mousem;          ///< 'mousemodel'
extern int p_mousemev;          ///< 'mousemoveevent'
extern int p_mousef;            ///< 'mousefocus'
extern int p_mh;                ///< 'mousehide'
extern char *p_mousescroll;     ///< 'mousescroll'
extern OptInt p_mousescroll_vert;
extern OptInt p_mousescroll_hor;
extern OptInt p_mouset;         ///< 'mousetime'
extern int p_more;              ///< 'more'
extern char *p_nf;              ///< 'nrformats'
extern Callback p_opfunc;       ///< 'operatorfunc'
extern char *p_para;            ///< 'paragraphs'
extern int p_paste;             ///< 'paste'
extern Callback p_pex;          ///< 'patchexpr'
extern char *p_pm;              ///< 'patchmode'
extern char *p_path;            ///< 'path'
extern char *p_cdpath;          ///< 'cdpath'
extern int p_pi;                ///< 'preserveindent'
extern OptInt p_pyx;            ///< 'pyxversion'
extern char *p_qe;              ///< 'quoteescape'
extern int p_ro;                ///< 'readonly'
extern char *p_rdb;             ///< 'redrawdebug'
extern unsigned rdb_flags;
extern OptInt p_rdt;            ///< 'redrawtime'
extern OptInt p_re;             ///< 'regexpengine'
extern OptInt p_report;         ///< 'report'
extern char *p_pvp;             ///< 'previewpopup'
extern OptInt p_pvh;            ///< 'previewheight'
extern OptInt p_chi;            ///< 'chistory'
extern int p_ari;               ///< 'allowrevins'
extern int p_ri;                ///< 'revins'
extern int p_ru;                ///< 'ruler'
extern char *p_ruf;             ///< 'rulerformat'
extern char *p_plf;             ///< 'packlockfile'
extern char *p_pp;              ///< 'packpath'
extern Callback p_qftf;         ///< 'quickfixtextfunc'
extern char *p_rtp;             ///< 'runtimepath'
extern OptInt p_scbk;           ///< 'scrollback'
extern OptInt p_sj;             ///< 'scrolljump'
extern OptInt p_so;             ///< 'scrolloff'
extern OptInt p_sop;            ///< 'scrolloffpad'
extern char *p_sbo;             ///< 'scrollopt'
extern char *p_sections;        ///< 'sections'
extern int p_secure;            ///< 'secure'
extern char *p_sel;             ///< 'selection'
extern char *p_slm;             ///< 'selectmode'
extern char *p_ssop;            ///< 'sessionoptions'
extern unsigned ssop_flags;
extern char *p_sh;              ///< 'shell'
extern char *p_shcf;            ///< 'shellcmdflag'
extern char *p_sp;              ///< 'shellpipe'
extern char *p_shq;             ///< 'shellquote'
extern char *p_sxq;             ///< 'shellxquote'
extern char *p_sxe;             ///< 'shellxescape'
extern char *p_srr;             ///< 'shellredir'
extern int p_stmp;              ///< 'shelltemp'
#ifdef BACKSLASH_IN_FILENAME
extern int p_ssl;               ///< 'shellslash'
#endif
extern char *p_stl;             ///< 'statusline'
extern char *p_wbr;             ///< 'winbar'
extern int p_sr;                ///< 'shiftround'
extern OptInt p_sw;             ///< 'shiftwidth'
extern char *p_shm;             ///< 'shortmess'
extern char *p_sbr;             ///< 'showbreak'
extern int p_sc;                ///< 'showcmd'
extern char *p_sloc;            ///< 'showcmdloc'
extern int p_sft;               ///< 'showfulltag'
extern int p_sm;                ///< 'showmatch'
extern int p_smd;               ///< 'showmode'
extern OptInt p_ss;             ///< 'sidescroll'
extern OptInt p_siso;           ///< 'sidescrolloff'
extern int p_scs;               ///< 'smartcase'
extern int p_si;                ///< 'smartindent'
extern int p_sta;               ///< 'smarttab'
extern OptInt p_sts;            ///< 'softtabstop'
extern int p_sb;                ///< 'splitbelow'
extern char *p_sua;             ///< 'suffixesadd'
extern int p_swf;               ///< 'swapfile'
extern OptInt p_smc;            ///< 'synmaxcol'
extern OptInt p_tpm;            ///< 'tabpagemax'
extern char *p_tal;             ///< 'tabline'
extern char *p_tpf;             ///< 'termpastefilter'
extern unsigned tpf_flags;      ///< flags from 'termpastefilter'
extern Callback p_tfu;          ///< 'tagfunc'
extern char *p_spc;             ///< 'spellcapcheck'
extern char *p_spf;             ///< 'spellfile'
extern char *p_spl;             ///< 'spelllang'
extern char *p_spo;             ///< 'spelloptions'
extern unsigned spo_flags;
extern char *p_sps;             ///< 'spellsuggest'
extern int p_spr;               ///< 'splitright'
extern int p_sol;               ///< 'startofline'
extern char *p_su;              ///< 'suffixes'
extern char *p_swb;             ///< 'switchbuf'
extern unsigned swb_flags;
extern char *p_spk;             ///< 'splitkeep'
extern char *p_syn;             ///< 'syntax'
extern char *p_tcl;             ///< 'tabclose'
extern unsigned tcl_flags;      ///< flags from 'tabclose'
extern OptInt p_ts;             ///< 'tabstop'
extern int p_tbs;               ///< 'tagbsearch'
extern char *p_tc;              ///< 'tagcase'
extern unsigned tc_flags;       ///< flags from 'tagcase'
extern OptInt p_tl;             ///< 'taglength'
extern int p_tr;                ///< 'tagrelative'
extern char *p_tags;            ///< 'tags'
extern int p_tgst;              ///< 'tagstack'
extern int p_tbidi;             ///< 'termbidi'
extern OptInt p_tw;             ///< 'textwidth'
extern int p_to;                ///< 'tildeop'
extern int p_timeout;           ///< 'timeout'
extern OptInt p_tm;             ///< 'timeoutlen'
extern int p_title;             ///< 'title'
extern OptInt p_titlelen;       ///< 'titlelen'
extern char *p_titleold;        ///< 'titleold'
extern char *p_titlestring;     ///< 'titlestring'
extern char *p_tsr;             ///< 'thesaurus'
extern int p_tgc;               ///< 'termguicolors'
extern int p_ttimeout;          ///< 'ttimeout'
extern OptInt p_ttm;            ///< 'ttimeoutlen'
extern int p_tf;                ///< 'ttyfast'
extern char *p_udir;            ///< 'undodir'
extern int p_udf;               ///< 'undofile'
extern OptInt p_ul;             ///< 'undolevels'
extern OptInt p_ur;             ///< 'undoreload'
extern OptInt p_uc;             ///< 'updatecount'
extern OptInt p_ut;             ///< 'updatetime'
extern char *p_shada;           ///< 'shada'
extern char *p_shadafile;       ///< 'shadafile'
extern int p_termsync;          ///< 'termsync'
extern char *p_vsts;            ///< 'varsofttabstop'
extern char *p_vts;             ///< 'vartabstop'
extern char *p_vdir;            ///< 'viewdir'
extern char *p_vop;             ///< 'viewoptions'
extern unsigned vop_flags;      ///< uses OptSsopFlags
extern int p_vb;                ///< 'visualbell'
extern char *p_ve;              ///< 'virtualedit'
extern unsigned ve_flags;
extern OptInt p_verbose;        ///< 'verbose'
extern char *p_vfile;           ///< 'verbosefile'
extern int p_warn;              ///< 'warn'
extern char *p_wop;             ///< 'wildoptions'
extern unsigned wop_flags;
extern OptInt p_window;         ///< 'window'
extern char *p_wak;             ///< 'winaltkeys'
extern char *p_wig;             ///< 'wildignore'
extern char *p_ww;              ///< 'whichwrap'
extern OptInt p_wc;             ///< 'wildchar'
extern OptInt p_wcm;            ///< 'wildcharm'
extern int p_wic;               ///< 'wildignorecase'
extern char *p_wim;             ///< 'wildmode'
extern int p_wmnu;              ///< 'wildmenu'
extern char *p_winborder;       ///< 'winborder'
extern OptInt p_wh;             ///< 'winheight'
extern OptInt p_wmh;            ///< 'winminheight'
extern OptInt p_wmw;            ///< 'winminwidth'
extern OptInt p_wiw;            ///< 'winwidth'
extern OptInt p_wm;             ///< 'wrapmargin'
extern int p_ws;                ///< 'wrapscan'
extern int p_write;             ///< 'write'
extern int p_wa;                ///< 'writeany'
extern int p_wb;                ///< 'writebackup'
extern OptInt p_wd;             ///< 'writedelay'
extern int p_cdh;               ///< 'cdhome'

// Value for b_p_ul indicating the global value must be used.
#define NO_LOCAL_UNDOLEVEL (-123456)

// Buffer for an option-set error message. Large enough to list an option's valid values (see
// opt_values_err()); the value is appended separately into IObuff.
#define ERR_BUFLEN 256

#define SB_MAX 1000000  // Maximum 'scrollback' value.

#define MAX_NUMBERWIDTH 20      // used for 'numberwidth'

// Maximum 'statuscolumn' width: number + sign + fold columns
#define MAX_STCWIDTH MAX_NUMBERWIDTH + SIGN_SHOW_MAX * SIGN_WIDTH + 9

#define TABSTOP_MAX 9999

#define SCL_NO  -1  // 'signcolumn' set to "no"
#define SCL_NUM -2  // 'signcolumn' set to "number"
