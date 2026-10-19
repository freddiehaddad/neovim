#pragma once

#include "nvim/eval/typval_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/option_defs.h"
#include "nvim/os/os_defs.h"
#include "nvim/sign_defs.h"
#include "nvim/statusline_defs.h"
#include "nvim/types_defs.h"

#include "option_vars.generated.h"  // NOLINT(build/include_defs)

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
#ifdef IN_OPTION_C
char *p_vfile = empty_string_option;  ///< used before options are initialized
#else
extern char *p_vfile;           ///< 'verbosefile'
#endif
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
