#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "nvim/api/private/defs.h"
#include "nvim/cmdexpand_defs.h"
#include "nvim/regexp_defs.h"
#include "nvim/types_defs.h"

typedef enum {
  kOptInvalid = -1,
  kOptAleph = 0,
  kOptAllowrevins = 1,
  kOptAmbiwidth = 2,
  kOptArabic = 3,
  kOptArabicshape = 4,
  kOptAutochdir = 5,
  kOptAutocomplete = 6,
  kOptAutocompletedelay = 7,
  kOptAutocompletetimeout = 8,
  kOptAutoindent = 9,
  kOptAutoread = 10,
  kOptAutowrite = 11,
  kOptAutowriteall = 12,
  kOptBackground = 13,
  kOptBackspace = 14,
  kOptBackup = 15,
  kOptBackupcopy = 16,
  kOptBackupdir = 17,
  kOptBackupext = 18,
  kOptBackupskip = 19,
  kOptBelloff = 20,
  kOptBinary = 21,
  kOptBomb = 22,
  kOptBreakat = 23,
  kOptBreakindent = 24,
  kOptBreakindentopt = 25,
  kOptBrowsedir = 26,
  kOptBufhidden = 27,
  kOptBuflisted = 28,
  kOptBuftype = 29,
  kOptBusy = 30,
  kOptCasemap = 31,
  kOptCdhome = 32,
  kOptCdpath = 33,
  kOptCedit = 34,
  kOptChannel = 35,
  kOptCharconvert = 36,
  kOptChistory = 37,
  kOptCindent = 38,
  kOptCinkeys = 39,
  kOptCinoptions = 40,
  kOptCinscopedecls = 41,
  kOptCinwords = 42,
  kOptClipboard = 43,
  kOptCmdheight = 44,
  kOptCmdwinheight = 45,
  kOptColorcolumn = 46,
  kOptColumns = 47,
  kOptComments = 48,
  kOptCommentstring = 49,
  kOptCompatible = 50,
  kOptComplete = 51,
  kOptCompletefunc = 52,
  kOptCompleteitemalign = 53,
  kOptCompleteopt = 54,
  kOptCompleteslash = 55,
  kOptCompletetimeout = 56,
  kOptConcealcursor = 57,
  kOptConceallevel = 58,
  kOptConfirm = 59,
  kOptCopyindent = 60,
  kOptCpoptions = 61,
  kOptCursorbind = 62,
  kOptCursorcolumn = 63,
  kOptCursorline = 64,
  kOptCursorlineopt = 65,
  kOptDebug = 66,
  kOptDefine = 67,
  kOptDelcombine = 68,
  kOptDictionary = 69,
  kOptDiff = 70,
  kOptDiffanchors = 71,
  kOptDiffexpr = 72,
  kOptDiffopt = 73,
  kOptDigraph = 74,
  kOptDirectory = 75,
  kOptDisplay = 76,
  kOptEadirection = 77,
  kOptEdcompatible = 78,
  kOptEmoji = 79,
  kOptEncoding = 80,
  kOptEndoffile = 81,
  kOptEndofline = 82,
  kOptEqualalways = 83,
  kOptEqualprg = 84,
  kOptErrorbells = 85,
  kOptErrorfile = 86,
  kOptErrorformat = 87,
  kOptEventignore = 88,
  kOptEventignorewin = 89,
  kOptExpandtab = 90,
  kOptExrc = 91,
  kOptFileencoding = 92,
  kOptFileencodings = 93,
  kOptFileformat = 94,
  kOptFileformats = 95,
  kOptFileignorecase = 96,
  kOptFiletype = 97,
  kOptFillchars = 98,
  kOptFindfunc = 99,
  kOptFixendofline = 100,
  kOptFoldclose = 101,
  kOptFoldcolumn = 102,
  kOptFoldenable = 103,
  kOptFoldexpr = 104,
  kOptFoldignore = 105,
  kOptFoldlevel = 106,
  kOptFoldlevelstart = 107,
  kOptFoldmarker = 108,
  kOptFoldmethod = 109,
  kOptFoldminlines = 110,
  kOptFoldnestmax = 111,
  kOptFoldopen = 112,
  kOptFoldtext = 113,
  kOptFollow = 114,
  kOptFormatexpr = 115,
  kOptFormatlistpat = 116,
  kOptFormatoptions = 117,
  kOptFormatprg = 118,
  kOptFsync = 119,
  kOptGdefault = 120,
  kOptGrepformat = 121,
  kOptGrepprg = 122,
  kOptGuicursor = 123,
  kOptGuifont = 124,
  kOptGuifontwide = 125,
  kOptGuioptions = 126,
  kOptGuitablabel = 127,
  kOptGuitabtooltip = 128,
  kOptHelpfile = 129,
  kOptHelpheight = 130,
  kOptHelplang = 131,
  kOptHidden = 132,
  kOptHighlight = 133,
  kOptHistory = 134,
  kOptHkmap = 135,
  kOptHkmapp = 136,
  kOptHlsearch = 137,
  kOptIcon = 138,
  kOptIconstring = 139,
  kOptIgnorecase = 140,
  kOptImcmdline = 141,
  kOptImdisable = 142,
  kOptIminsert = 143,
  kOptImsearch = 144,
  kOptInccommand = 145,
  kOptInclude = 146,
  kOptIncludeexpr = 147,
  kOptIncsearch = 148,
  kOptIndentexpr = 149,
  kOptIndentkeys = 150,
  kOptInfercase = 151,
  kOptInsertmode = 152,
  kOptIsfname = 153,
  kOptIsident = 154,
  kOptIskeyword = 155,
  kOptIsprint = 156,
  kOptJoinspaces = 157,
  kOptJumpoptions = 158,
  kOptKeymap = 159,
  kOptKeymodel = 160,
  kOptKeywordprg = 161,
  kOptLangmap = 162,
  kOptLangmenu = 163,
  kOptLangnoremap = 164,
  kOptLangremap = 165,
  kOptLaststatus = 166,
  kOptLazyredraw = 167,
  kOptLhistory = 168,
  kOptLinebreak = 169,
  kOptLines = 170,
  kOptLinespace = 171,
  kOptLisp = 172,
  kOptLispoptions = 173,
  kOptLispwords = 174,
  kOptList = 175,
  kOptListchars = 176,
  kOptLoadplugins = 177,
  kOptMagic = 178,
  kOptMakeef = 179,
  kOptMakeencoding = 180,
  kOptMakeprg = 181,
  kOptMatchpairs = 182,
  kOptMatchtime = 183,
  kOptMaxcombine = 184,
  kOptMaxfuncdepth = 185,
  kOptMaxmapdepth = 186,
  kOptMaxmempattern = 187,
  kOptMaxsearchcount = 188,
  kOptMenuitems = 189,
  kOptMessagesopt = 190,
  kOptMkspellmem = 191,
  kOptModeline = 192,
  kOptModelineexpr = 193,
  kOptModelines = 194,
  kOptModifiable = 195,
  kOptModified = 196,
  kOptMore = 197,
  kOptMouse = 198,
  kOptMousefocus = 199,
  kOptMousehide = 200,
  kOptMousemodel = 201,
  kOptMousemoveevent = 202,
  kOptMousescroll = 203,
  kOptMouseshape = 204,
  kOptMousetime = 205,
  kOptNrformats = 206,
  kOptNumber = 207,
  kOptNumberwidth = 208,
  kOptOmnifunc = 209,
  kOptOpendevice = 210,
  kOptOperatorfunc = 211,
  kOptPacklockfile = 212,
  kOptPackpath = 213,
  kOptParagraphs = 214,
  kOptPaste = 215,
  kOptPastetoggle = 216,
  kOptPatchexpr = 217,
  kOptPatchmode = 218,
  kOptPath = 219,
  kOptPreserveindent = 220,
  kOptPreviewheight = 221,
  kOptPreviewpopup = 222,
  kOptPreviewwindow = 223,
  kOptPrompt = 224,
  kOptPumblend = 225,
  kOptPumborder = 226,
  kOptPumheight = 227,
  kOptPummaxwidth = 228,
  kOptPumwidth = 229,
  kOptPyxversion = 230,
  kOptQuickfixtextfunc = 231,
  kOptQuoteescape = 232,
  kOptReadonly = 233,
  kOptRedrawdebug = 234,
  kOptRedrawtime = 235,
  kOptRegexpengine = 236,
  kOptRelativenumber = 237,
  kOptRemap = 238,
  kOptReport = 239,
  kOptRevins = 240,
  kOptRightleft = 241,
  kOptRightleftcmd = 242,
  kOptRuler = 243,
  kOptRulerformat = 244,
  kOptRuntimepath = 245,
  kOptScroll = 246,
  kOptScrollback = 247,
  kOptScrollbind = 248,
  kOptScrolljump = 249,
  kOptScrolloff = 250,
  kOptScrolloffpad = 251,
  kOptScrollopt = 252,
  kOptSections = 253,
  kOptSecure = 254,
  kOptSelection = 255,
  kOptSelectmode = 256,
  kOptSessionoptions = 257,
  kOptShada = 258,
  kOptShadafile = 259,
  kOptShell = 260,
  kOptShellcmdflag = 261,
  kOptShellpipe = 262,
  kOptShellquote = 263,
  kOptShellredir = 264,
  kOptShellslash = 265,
  kOptShelltemp = 266,
  kOptShellxescape = 267,
  kOptShellxquote = 268,
  kOptShiftround = 269,
  kOptShiftwidth = 270,
  kOptShortmess = 271,
  kOptShowbreak = 272,
  kOptShowcmd = 273,
  kOptShowcmdloc = 274,
  kOptShowfulltag = 275,
  kOptShowmatch = 276,
  kOptShowmode = 277,
  kOptShowtabline = 278,
  kOptSidescroll = 279,
  kOptSidescrolloff = 280,
  kOptSigncolumn = 281,
  kOptSmartcase = 282,
  kOptSmartindent = 283,
  kOptSmarttab = 284,
  kOptSmoothscroll = 285,
  kOptSofttabstop = 286,
  kOptSpell = 287,
  kOptSpellcapcheck = 288,
  kOptSpellfile = 289,
  kOptSpelllang = 290,
  kOptSpelloptions = 291,
  kOptSpellsuggest = 292,
  kOptSplitbelow = 293,
  kOptSplitkeep = 294,
  kOptSplitright = 295,
  kOptStartofline = 296,
  kOptStatuscolumn = 297,
  kOptStatusline = 298,
  kOptSuffixes = 299,
  kOptSuffixesadd = 300,
  kOptSwapfile = 301,
  kOptSwitchbuf = 302,
  kOptSynmaxcol = 303,
  kOptSyntax = 304,
  kOptTabclose = 305,
  kOptTabline = 306,
  kOptTabpagemax = 307,
  kOptTabstop = 308,
  kOptTagbsearch = 309,
  kOptTagcase = 310,
  kOptTagfunc = 311,
  kOptTaglength = 312,
  kOptTagrelative = 313,
  kOptTags = 314,
  kOptTagstack = 315,
  kOptTermbidi = 316,
  kOptTermencoding = 317,
  kOptTermguicolors = 318,
  kOptTermpastefilter = 319,
  kOptTermsync = 320,
  kOptTerse = 321,
  kOptTextwidth = 322,
  kOptThesaurus = 323,
  kOptThesaurusfunc = 324,
  kOptTildeop = 325,
  kOptTimeout = 326,
  kOptTimeoutlen = 327,
  kOptTitle = 328,
  kOptTitlelen = 329,
  kOptTitleold = 330,
  kOptTitlestring = 331,
  kOptTtimeout = 332,
  kOptTtimeoutlen = 333,
  kOptTtyfast = 334,
  kOptUndodir = 335,
  kOptUndofile = 336,
  kOptUndolevels = 337,
  kOptUndoreload = 338,
  kOptUpdatecount = 339,
  kOptUpdatetime = 340,
  kOptVarsofttabstop = 341,
  kOptVartabstop = 342,
  kOptVerbose = 343,
  kOptVerbosefile = 344,
  kOptViewdir = 345,
  kOptViewoptions = 346,
  kOptVirtualedit = 347,
  kOptVisualbell = 348,
  kOptWarn = 349,
  kOptWhichwrap = 350,
  kOptWildchar = 351,
  kOptWildcharm = 352,
  kOptWildignore = 353,
  kOptWildignorecase = 354,
  kOptWildmenu = 355,
  kOptWildmode = 356,
  kOptWildoptions = 357,
  kOptWinaltkeys = 358,
  kOptWinbar = 359,
  kOptWinblend = 360,
  kOptWinborder = 361,
  kOptWindow = 362,
  kOptWinfixbuf = 363,
  kOptWinfixheight = 364,
  kOptWinfixwidth = 365,
  kOptWinheight = 366,
  kOptWinhighlight = 367,
  kOptWinminheight = 368,
  kOptWinminwidth = 369,
  kOptWinpinned = 370,
  kOptWinwidth = 371,
  kOptWrap = 372,
  kOptWrapmargin = 373,
  kOptWrapscan = 374,
  kOptWrite = 375,
  kOptWriteany = 376,
  kOptWritebackup = 377,
  kOptWritedelay = 378,
  // Option count
#define kOptCount 379
} OptIndex;

typedef enum {
  kGlobalOptInvalid = -1,
  kGlobalOptAleph = 0,
  kGlobalOptAllowrevins = 1,
  kGlobalOptAmbiwidth = 2,
  kGlobalOptArabicshape = 3,
  kGlobalOptAutochdir = 4,
  kGlobalOptAutocomplete = 5,
  kGlobalOptAutocompletedelay = 6,
  kGlobalOptAutocompletetimeout = 7,
  kGlobalOptAutoread = 8,
  kGlobalOptAutowrite = 9,
  kGlobalOptAutowriteall = 10,
  kGlobalOptBackground = 11,
  kGlobalOptBackspace = 12,
  kGlobalOptBackup = 13,
  kGlobalOptBackupcopy = 14,
  kGlobalOptBackupdir = 15,
  kGlobalOptBackupext = 16,
  kGlobalOptBackupskip = 17,
  kGlobalOptBelloff = 18,
  kGlobalOptBreakat = 19,
  kGlobalOptBrowsedir = 20,
  kGlobalOptCasemap = 21,
  kGlobalOptCdhome = 22,
  kGlobalOptCdpath = 23,
  kGlobalOptCedit = 24,
  kGlobalOptCharconvert = 25,
  kGlobalOptChistory = 26,
  kGlobalOptClipboard = 27,
  kGlobalOptCmdheight = 28,
  kGlobalOptCmdwinheight = 29,
  kGlobalOptColumns = 30,
  kGlobalOptCompatible = 31,
  kGlobalOptCompleteitemalign = 32,
  kGlobalOptCompleteopt = 33,
  kGlobalOptCompletetimeout = 34,
  kGlobalOptConfirm = 35,
  kGlobalOptCpoptions = 36,
  kGlobalOptDebug = 37,
  kGlobalOptDefine = 38,
  kGlobalOptDelcombine = 39,
  kGlobalOptDictionary = 40,
  kGlobalOptDiffanchors = 41,
  kGlobalOptDiffexpr = 42,
  kGlobalOptDiffopt = 43,
  kGlobalOptDigraph = 44,
  kGlobalOptDirectory = 45,
  kGlobalOptDisplay = 46,
  kGlobalOptEadirection = 47,
  kGlobalOptEdcompatible = 48,
  kGlobalOptEmoji = 49,
  kGlobalOptEncoding = 50,
  kGlobalOptEqualalways = 51,
  kGlobalOptEqualprg = 52,
  kGlobalOptErrorbells = 53,
  kGlobalOptErrorfile = 54,
  kGlobalOptErrorformat = 55,
  kGlobalOptEventignore = 56,
  kGlobalOptExrc = 57,
  kGlobalOptFileencodings = 58,
  kGlobalOptFileformats = 59,
  kGlobalOptFileignorecase = 60,
  kGlobalOptFillchars = 61,
  kGlobalOptFindfunc = 62,
  kGlobalOptFoldclose = 63,
  kGlobalOptFoldlevelstart = 64,
  kGlobalOptFoldopen = 65,
  kGlobalOptFormatprg = 66,
  kGlobalOptFsync = 67,
  kGlobalOptGdefault = 68,
  kGlobalOptGrepformat = 69,
  kGlobalOptGrepprg = 70,
  kGlobalOptGuicursor = 71,
  kGlobalOptGuifont = 72,
  kGlobalOptGuifontwide = 73,
  kGlobalOptGuioptions = 74,
  kGlobalOptGuitablabel = 75,
  kGlobalOptGuitabtooltip = 76,
  kGlobalOptHelpfile = 77,
  kGlobalOptHelpheight = 78,
  kGlobalOptHelplang = 79,
  kGlobalOptHidden = 80,
  kGlobalOptHighlight = 81,
  kGlobalOptHistory = 82,
  kGlobalOptHkmap = 83,
  kGlobalOptHkmapp = 84,
  kGlobalOptHlsearch = 85,
  kGlobalOptIcon = 86,
  kGlobalOptIconstring = 87,
  kGlobalOptIgnorecase = 88,
  kGlobalOptImcmdline = 89,
  kGlobalOptImdisable = 90,
  kGlobalOptInccommand = 91,
  kGlobalOptInclude = 92,
  kGlobalOptIncsearch = 93,
  kGlobalOptInsertmode = 94,
  kGlobalOptIsfname = 95,
  kGlobalOptIsident = 96,
  kGlobalOptIsprint = 97,
  kGlobalOptJoinspaces = 98,
  kGlobalOptJumpoptions = 99,
  kGlobalOptKeymodel = 100,
  kGlobalOptKeywordprg = 101,
  kGlobalOptLangmap = 102,
  kGlobalOptLangmenu = 103,
  kGlobalOptLangnoremap = 104,
  kGlobalOptLangremap = 105,
  kGlobalOptLaststatus = 106,
  kGlobalOptLazyredraw = 107,
  kGlobalOptLines = 108,
  kGlobalOptLinespace = 109,
  kGlobalOptLispwords = 110,
  kGlobalOptListchars = 111,
  kGlobalOptLoadplugins = 112,
  kGlobalOptMagic = 113,
  kGlobalOptMakeef = 114,
  kGlobalOptMakeencoding = 115,
  kGlobalOptMakeprg = 116,
  kGlobalOptMatchtime = 117,
  kGlobalOptMaxcombine = 118,
  kGlobalOptMaxfuncdepth = 119,
  kGlobalOptMaxmapdepth = 120,
  kGlobalOptMaxmempattern = 121,
  kGlobalOptMaxsearchcount = 122,
  kGlobalOptMenuitems = 123,
  kGlobalOptMessagesopt = 124,
  kGlobalOptMkspellmem = 125,
  kGlobalOptModelineexpr = 126,
  kGlobalOptModelines = 127,
  kGlobalOptMore = 128,
  kGlobalOptMouse = 129,
  kGlobalOptMousefocus = 130,
  kGlobalOptMousehide = 131,
  kGlobalOptMousemodel = 132,
  kGlobalOptMousemoveevent = 133,
  kGlobalOptMousescroll = 134,
  kGlobalOptMouseshape = 135,
  kGlobalOptMousetime = 136,
  kGlobalOptOpendevice = 137,
  kGlobalOptOperatorfunc = 138,
  kGlobalOptPacklockfile = 139,
  kGlobalOptPackpath = 140,
  kGlobalOptParagraphs = 141,
  kGlobalOptPaste = 142,
  kGlobalOptPastetoggle = 143,
  kGlobalOptPatchexpr = 144,
  kGlobalOptPatchmode = 145,
  kGlobalOptPath = 146,
  kGlobalOptPreviewheight = 147,
  kGlobalOptPreviewpopup = 148,
  kGlobalOptPrompt = 149,
  kGlobalOptPumblend = 150,
  kGlobalOptPumborder = 151,
  kGlobalOptPumheight = 152,
  kGlobalOptPummaxwidth = 153,
  kGlobalOptPumwidth = 154,
  kGlobalOptPyxversion = 155,
  kGlobalOptQuickfixtextfunc = 156,
  kGlobalOptRedrawdebug = 157,
  kGlobalOptRedrawtime = 158,
  kGlobalOptRegexpengine = 159,
  kGlobalOptRemap = 160,
  kGlobalOptReport = 161,
  kGlobalOptRevins = 162,
  kGlobalOptRuler = 163,
  kGlobalOptRulerformat = 164,
  kGlobalOptRuntimepath = 165,
  kGlobalOptScrolljump = 166,
  kGlobalOptScrolloff = 167,
  kGlobalOptScrolloffpad = 168,
  kGlobalOptScrollopt = 169,
  kGlobalOptSections = 170,
  kGlobalOptSecure = 171,
  kGlobalOptSelection = 172,
  kGlobalOptSelectmode = 173,
  kGlobalOptSessionoptions = 174,
  kGlobalOptShada = 175,
  kGlobalOptShadafile = 176,
  kGlobalOptShell = 177,
  kGlobalOptShellcmdflag = 178,
  kGlobalOptShellpipe = 179,
  kGlobalOptShellquote = 180,
  kGlobalOptShellredir = 181,
  kGlobalOptShellslash = 182,
  kGlobalOptShelltemp = 183,
  kGlobalOptShellxescape = 184,
  kGlobalOptShellxquote = 185,
  kGlobalOptShiftround = 186,
  kGlobalOptShortmess = 187,
  kGlobalOptShowbreak = 188,
  kGlobalOptShowcmd = 189,
  kGlobalOptShowcmdloc = 190,
  kGlobalOptShowfulltag = 191,
  kGlobalOptShowmatch = 192,
  kGlobalOptShowmode = 193,
  kGlobalOptShowtabline = 194,
  kGlobalOptSidescroll = 195,
  kGlobalOptSidescrolloff = 196,
  kGlobalOptSmartcase = 197,
  kGlobalOptSmarttab = 198,
  kGlobalOptSpellsuggest = 199,
  kGlobalOptSplitbelow = 200,
  kGlobalOptSplitkeep = 201,
  kGlobalOptSplitright = 202,
  kGlobalOptStartofline = 203,
  kGlobalOptStatusline = 204,
  kGlobalOptSuffixes = 205,
  kGlobalOptSwitchbuf = 206,
  kGlobalOptTabclose = 207,
  kGlobalOptTabline = 208,
  kGlobalOptTabpagemax = 209,
  kGlobalOptTagbsearch = 210,
  kGlobalOptTagcase = 211,
  kGlobalOptTaglength = 212,
  kGlobalOptTagrelative = 213,
  kGlobalOptTags = 214,
  kGlobalOptTagstack = 215,
  kGlobalOptTermbidi = 216,
  kGlobalOptTermencoding = 217,
  kGlobalOptTermguicolors = 218,
  kGlobalOptTermpastefilter = 219,
  kGlobalOptTermsync = 220,
  kGlobalOptTerse = 221,
  kGlobalOptThesaurus = 222,
  kGlobalOptThesaurusfunc = 223,
  kGlobalOptTildeop = 224,
  kGlobalOptTimeout = 225,
  kGlobalOptTimeoutlen = 226,
  kGlobalOptTitle = 227,
  kGlobalOptTitlelen = 228,
  kGlobalOptTitleold = 229,
  kGlobalOptTitlestring = 230,
  kGlobalOptTtimeout = 231,
  kGlobalOptTtimeoutlen = 232,
  kGlobalOptTtyfast = 233,
  kGlobalOptUndodir = 234,
  kGlobalOptUndolevels = 235,
  kGlobalOptUndoreload = 236,
  kGlobalOptUpdatecount = 237,
  kGlobalOptUpdatetime = 238,
  kGlobalOptVerbose = 239,
  kGlobalOptVerbosefile = 240,
  kGlobalOptViewdir = 241,
  kGlobalOptViewoptions = 242,
  kGlobalOptVirtualedit = 243,
  kGlobalOptVisualbell = 244,
  kGlobalOptWarn = 245,
  kGlobalOptWhichwrap = 246,
  kGlobalOptWildchar = 247,
  kGlobalOptWildcharm = 248,
  kGlobalOptWildignore = 249,
  kGlobalOptWildignorecase = 250,
  kGlobalOptWildmenu = 251,
  kGlobalOptWildmode = 252,
  kGlobalOptWildoptions = 253,
  kGlobalOptWinaltkeys = 254,
  kGlobalOptWinbar = 255,
  kGlobalOptWinborder = 256,
  kGlobalOptWindow = 257,
  kGlobalOptWinheight = 258,
  kGlobalOptWinminheight = 259,
  kGlobalOptWinminwidth = 260,
  kGlobalOptWinwidth = 261,
  kGlobalOptWrapscan = 262,
  kGlobalOptWrite = 263,
  kGlobalOptWriteany = 264,
  kGlobalOptWritebackup = 265,
  kGlobalOptWritedelay = 266,
  // Global option count
#define kGlobalOptCount 267
} GlobalOptIndex;

typedef enum {
  kBufOptInvalid = -1,
  kBufOptAutocomplete = 0,
  kBufOptAutoindent = 1,
  kBufOptAutoread = 2,
  kBufOptBackupcopy = 3,
  kBufOptBinary = 4,
  kBufOptBomb = 5,
  kBufOptBufhidden = 6,
  kBufOptBuflisted = 7,
  kBufOptBuftype = 8,
  kBufOptBusy = 9,
  kBufOptChannel = 10,
  kBufOptCindent = 11,
  kBufOptCinkeys = 12,
  kBufOptCinoptions = 13,
  kBufOptCinscopedecls = 14,
  kBufOptCinwords = 15,
  kBufOptComments = 16,
  kBufOptCommentstring = 17,
  kBufOptComplete = 18,
  kBufOptCompletefunc = 19,
  kBufOptCompleteopt = 20,
  kBufOptCompleteslash = 21,
  kBufOptCopyindent = 22,
  kBufOptDefine = 23,
  kBufOptDictionary = 24,
  kBufOptDiffanchors = 25,
  kBufOptEndoffile = 26,
  kBufOptEndofline = 27,
  kBufOptEqualprg = 28,
  kBufOptErrorformat = 29,
  kBufOptExpandtab = 30,
  kBufOptFileencoding = 31,
  kBufOptFileformat = 32,
  kBufOptFiletype = 33,
  kBufOptFindfunc = 34,
  kBufOptFixendofline = 35,
  kBufOptFollow = 36,
  kBufOptFormatexpr = 37,
  kBufOptFormatlistpat = 38,
  kBufOptFormatoptions = 39,
  kBufOptFormatprg = 40,
  kBufOptFsync = 41,
  kBufOptGrepformat = 42,
  kBufOptGrepprg = 43,
  kBufOptIminsert = 44,
  kBufOptImsearch = 45,
  kBufOptInclude = 46,
  kBufOptIncludeexpr = 47,
  kBufOptIndentexpr = 48,
  kBufOptIndentkeys = 49,
  kBufOptInfercase = 50,
  kBufOptIskeyword = 51,
  kBufOptKeymap = 52,
  kBufOptKeywordprg = 53,
  kBufOptLisp = 54,
  kBufOptLispoptions = 55,
  kBufOptLispwords = 56,
  kBufOptMakeencoding = 57,
  kBufOptMakeprg = 58,
  kBufOptMatchpairs = 59,
  kBufOptModeline = 60,
  kBufOptModifiable = 61,
  kBufOptModified = 62,
  kBufOptNrformats = 63,
  kBufOptOmnifunc = 64,
  kBufOptPath = 65,
  kBufOptPreserveindent = 66,
  kBufOptQuoteescape = 67,
  kBufOptReadonly = 68,
  kBufOptScrollback = 69,
  kBufOptShiftwidth = 70,
  kBufOptSmartindent = 71,
  kBufOptSofttabstop = 72,
  kBufOptSpellcapcheck = 73,
  kBufOptSpellfile = 74,
  kBufOptSpelllang = 75,
  kBufOptSpelloptions = 76,
  kBufOptSuffixesadd = 77,
  kBufOptSwapfile = 78,
  kBufOptSynmaxcol = 79,
  kBufOptSyntax = 80,
  kBufOptTabstop = 81,
  kBufOptTagcase = 82,
  kBufOptTagfunc = 83,
  kBufOptTags = 84,
  kBufOptTextwidth = 85,
  kBufOptThesaurus = 86,
  kBufOptThesaurusfunc = 87,
  kBufOptUndofile = 88,
  kBufOptUndolevels = 89,
  kBufOptVarsofttabstop = 90,
  kBufOptVartabstop = 91,
  kBufOptWrapmargin = 92,
  // Buf option count
#define kBufOptCount 93
} BufOptIndex;

typedef enum {
  kWinOptInvalid = -1,
  kWinOptArabic = 0,
  kWinOptBreakindent = 1,
  kWinOptBreakindentopt = 2,
  kWinOptColorcolumn = 3,
  kWinOptConcealcursor = 4,
  kWinOptConceallevel = 5,
  kWinOptCursorbind = 6,
  kWinOptCursorcolumn = 7,
  kWinOptCursorline = 8,
  kWinOptCursorlineopt = 9,
  kWinOptDiff = 10,
  kWinOptEventignorewin = 11,
  kWinOptFillchars = 12,
  kWinOptFoldcolumn = 13,
  kWinOptFoldenable = 14,
  kWinOptFoldexpr = 15,
  kWinOptFoldignore = 16,
  kWinOptFoldlevel = 17,
  kWinOptFoldmarker = 18,
  kWinOptFoldmethod = 19,
  kWinOptFoldminlines = 20,
  kWinOptFoldnestmax = 21,
  kWinOptFoldtext = 22,
  kWinOptLhistory = 23,
  kWinOptLinebreak = 24,
  kWinOptList = 25,
  kWinOptListchars = 26,
  kWinOptNumber = 27,
  kWinOptNumberwidth = 28,
  kWinOptPreviewwindow = 29,
  kWinOptRelativenumber = 30,
  kWinOptRightleft = 31,
  kWinOptRightleftcmd = 32,
  kWinOptScroll = 33,
  kWinOptScrollbind = 34,
  kWinOptScrolloff = 35,
  kWinOptScrolloffpad = 36,
  kWinOptShowbreak = 37,
  kWinOptSidescrolloff = 38,
  kWinOptSigncolumn = 39,
  kWinOptSmoothscroll = 40,
  kWinOptSpell = 41,
  kWinOptStatuscolumn = 42,
  kWinOptStatusline = 43,
  kWinOptVirtualedit = 44,
  kWinOptWinbar = 45,
  kWinOptWinblend = 46,
  kWinOptWinfixbuf = 47,
  kWinOptWinfixheight = 48,
  kWinOptWinfixwidth = 49,
  kWinOptWinhighlight = 50,
  kWinOptWinpinned = 51,
  kWinOptWrap = 52,
  // Win option count
#define kWinOptCount 53
} WinOptIndex;

typedef enum {
  kTabOptInvalid = -1,
  kTabOptCmdheight = 0,
  // Tab option count
#define kTabOptCount 1
} TabOptIndex;

/// Option flags.
typedef enum {
  kOptFlagExpand    = 1 << 0,  ///< Environment expansion.
                               ///< NOTE: kOptFlagExpand can never be used for local or hidden options.
  kOptFlagNoDefExp  = 1 << 1,  ///< Don't expand default value.
  kOptFlagNoDefault = 1 << 2,  ///< Don't set to default value.
  kOptFlagWasSet    = 1 << 3,  ///< Option has been set/reset.
  kOptFlagNoMkrc    = 1 << 4,  ///< Don't include in :mkvimrc output.
  kOptFlagUIOption  = 1 << 5,  ///< Send option to remote UI.
  kOptFlagRedrTabl  = 1 << 6,  ///< Redraw tabline.
  kOptFlagRedrStat  = 1 << 7,  ///< Redraw status lines.
  kOptFlagRedrWin   = 1 << 8,  ///< Redraw current window and recompute text.
  kOptFlagRedrBuf   = 1 << 9,  ///< Redraw current buffer and recompute text.
  kOptFlagRedrAll   = kOptFlagRedrBuf | kOptFlagRedrWin,  ///< Redraw all windows and recompute text.
  kOptFlagRedrClear = kOptFlagRedrAll | kOptFlagRedrStat,  ///< Clear and redraw all and recompute text.
  kOptFlagComma     = 1 << 10,  ///< Comma-separated list.
  kOptFlagOneComma  = (1 << 11) | kOptFlagComma,  ///< Comma-separated list that cannot have two consecutive commas.
  kOptFlagNoDup     = 1 << 12,  ///< Don't allow duplicate strings.
  kOptFlagFlagList  = 1 << 13,  ///< List of single-char flags.
  kOptFlagSecure    = 1 << 14,  ///< Cannot change in modelines, secure mode, or the sandbox.
  kOptFlagGettext   = 1 << 15,  ///< Expand default value with _().
  kOptFlagNoGlob    = 1 << 16,  ///< Do not use local value for global vimrc.
  kOptFlagNFname    = 1 << 17,  ///< Only normal file name chars allowed.
  kOptFlagInsecure  = 1 << 18,  ///< Value contains untrusted content; evaluate it in the sandbox.
  kOptFlagPriMkrc   = 1 << 19,  ///< Priority for :mkvimrc (setting option has side effects).
  kOptFlagCurswant  = 1 << 20,  ///< Update curswant required; not needed when there is a redraw flag.
  kOptFlagNDname    = 1 << 21,  ///< Only normal directory name chars allowed.
  kOptFlagHLOnly    = 1 << 22,  ///< Option only changes highlight, not text.
  kOptFlagMLE       = 1 << 23,  ///< Under control of 'modelineexpr'.
  kOptFlagFunc      = 1 << 24,  ///< Callback option: Lua function, or Vimscript funcref.
  kOptFlagColon     = 1 << 25,  ///< Values use colons to create sublists.
  kOptFlagExpr      = 1 << 26,  ///< Callback option: Vimscript expr, e.g. 'foldexpr'.
} OptFlags;

/// Scopes that an option can support.
typedef enum {
  kOptScopeGlobal = 0,  ///< Request global option value
  kOptScopeWin,      ///< Request window-local option value
  kOptScopeBuf,      ///< Request buffer-local option value
  kOptScopeTab,      ///< Request tabpage-local option value
} OptScope;
/// Always update this whenever a new option scope is added.
#define kOptScopeSize (kOptScopeTab + 1)
typedef uint8_t OptScopeFlags;

/// Value kind of one key in a dict option (see "schema" in options.lua).
typedef enum {
  kOptSchemaFlag,   ///< bare flag, e.g. 'diffopt' "filler"
  kOptSchemaNum,    ///< "key:N" non-negative number, e.g. 'diffopt' "context:4"
  kOptSchemaSNum,   ///< "key:N" signed number, e.g. 'breakindentopt' "shift:-2"
  kOptSchemaEnum,   ///< "key:val" where val is one of "enum_values", e.g. "algorithm:patience"
  kOptSchemaStr,    ///< "key:val" where val is a free string, e.g. 'previewpopup' "highlight:Foo"
} OptSchemaKind;

/// One key of a dict option, generated from its `dict` schema in options.lua.
/// Consumed by opt_strings_check(). A NULL "name" terminates the array.
typedef struct {
  const char *name;           ///< key name, without a trailing ':'
  OptSchemaKind kind;
  const char **enum_values;    ///< kOptSchemaEnum: NULL-terminated valid values; else NULL
} OptSchemaItem;

/// Schema for a dict option (`schema.dict` in options.lua).
typedef struct {
  FieldHashfn get_field;        ///< Keyset perfect-hash lookup, for opt_fill().
  const OptSchemaItem *schema;  ///< Grammar, for opt_strings_check() validation.
  size_t size;                  ///< sizeof the keyset, for opt_keyset() storage.
} OptDictSchema;

/// :set operator types
typedef enum {
  OP_NONE = 0,
  OP_ADDING,      ///< "opt+=arg"
  OP_PREPENDING,  ///< "opt^=arg"
  OP_REMOVING,    ///< "opt-=arg"
} set_op_T;

/// Arguments for validating an option value or applying it after storage.
typedef struct {
  /// Pointer to the option variable.  The variable can be an OptInt (numeric
  /// option), an int (boolean option), a char pointer (string option), or a Callback.
  /// Holds the old value during validation and the new value when applying it.
  void *os_varp;
  OptIndex os_idx;
  int os_flags;

  /// Old value of the option.
  Object os_oldval;
  /// New value of the option (not yet stored during validation).
  Object os_newval;

  /// Initially empty; validation may prepare data for the did_set callback.
  /// The callbacks agree on the union member and pointer type. A callback taking
  /// ownership of a pointer must clear it.
  union {
    void *ptr;
    unsigned flags;
  } os_prep;
  /// Frees non-NULL prepared data after a dry run, an error, or application.
  void (*os_prep_free)(void *);

  /// Option value was checked to be safe, no need to set kOptFlagInsecure
  /// Used for the 'keymap', 'filetype' and 'syntax' options.
  bool os_value_checked;
  /// Option value changed.  Used for the 'filetype' and 'syntax' options.
  bool os_value_changed;

  /// Used by the 'isident', 'iskeyword', 'isprint' and 'isfname' options.
  /// Set to true if the character table is modified when processing the
  /// option and need to be restored because of a failure.
  bool os_restore_chartab;

  /// If the value specified for an option is not valid and the error message
  /// is parameterized, then the "os_errbuf" buffer is used to store the error
  /// message (when it is not NULL).
  const CharBuf *os_errbuf;

  win_T *os_win;
  buf_T *os_buf;
} optset_T;

/// Check a candidate value without changing option variables or derived state,
/// or evaluating user code. Return an error message, or NULL on success.
/// May populate os_prep for the did_set callback.
typedef const char *(*opt_validate_cb_T)(optset_T *args);

/// Type for the callback function invoked after storing an option value to
/// apply it and update derived state.
///
/// Returns NULL on success, or an error message if the value could not be applied.
typedef const char *(*opt_did_set_cb_T)(optset_T *args);

/// Argument for the callback function (opt_expand_cb_T) invoked after a string
/// option value is expanded for cmdline completion.
typedef struct {
  /// Pointer to the option variable. It's always a string.
  char *oe_varp;
  OptIndex oe_idx;
  /// The original option value, escaped.
  char *oe_opt_value;

  /// true if using set+= instead of set=
  bool oe_append;
  /// true if we would like to add the original option value as the first choice.
  bool oe_include_orig_val;

  /// Regex from the cmdline, for matching potential options against.
  regmatch_T *oe_regmatch;
  /// The expansion context.
  expand_T *oe_xp;

  /// The full argument passed to :set. For example, if the user inputs
  /// ":set dip=icase,algorithm:my<Tab>", oe_xp->xp_pattern will only have
  /// "my", but oe_set_arg will contain the whole "icase,algorithm:my".
  char *oe_set_arg;
} optexpand_T;

/// Type for the callback function that is invoked when expanding possible
/// string option values during cmdline completion.
///
/// Strings in returned matches will be managed and freed by caller.
///
/// Returns OK if the expansion succeeded (numMatches and matches have to be
/// set). Otherwise returns FAIL.
///
/// Note: If returned FAIL or *numMatches is 0, *matches will NOT be freed by
/// caller.
typedef int (*opt_expand_cb_T)(optexpand_T *args, int *numMatches, char ***matches);

typedef struct {
  char *fullname;                    ///< full option name
  char *shortname;                   ///< permissible abbreviation
  uint32_t flags;                    ///< see above
  ObjectType type;                   ///< option type
  OptScopeFlags scope_flags;         ///< option scope flags, see OptScope
  void *var;                         ///< global option: pointer to variable;
                                     ///< window-local option: NULL;
                                     ///< buffer-local option: global value
  unsigned *flags_var;
  ssize_t scope_idx[kOptScopeSize];  ///< index of option at every scope.
  bool immutable;                    ///< option is immutable, trying to set it will give an error.

  const char **values;               ///< possible values for string options
  const size_t values_len;           ///< length of values array

  /// Grammar of a dict option ("schema.dict" in options.lua); NULL otherwise.
  const OptSchemaItem *schema;

  /// Validate a candidate value before storing it, without side effects.
  opt_validate_cb_T opt_validate_cb;

  /// Apply the stored value. Some handlers still perform option-specific validation.
  opt_did_set_cb_T opt_did_set_cb;

  /// callback function to invoke when expanding possible values on the
  /// cmdline. Only useful for string options.
  opt_expand_cb_T opt_expand_cb;

  Object def_val;                    ///< default value
  sctx_T script_ctx;                 ///< script in which the option was last set
} vimoption_T;
