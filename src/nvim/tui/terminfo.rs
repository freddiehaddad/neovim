//! Built-in fallback terminfo entries and database loader (`src/nvim/tui/terminfo.c`).

use crate::charset::{NUL, kv_transstr};
use crate::strings::vim_snprintf;
use crate::tui::termkey::driver_ti::{
    TerminfoEntry, K_TERM_COUNT, K_TERM_KEY_COUNT, K_TERMINFO_FUNC_KEY_MAX,
};
use std::borrow::Cow;
use std::ffi::{CStr, c_char, c_int, c_long};
use std::io::Read;
use std::ptr;

pub const K_TERM_CARRIAGE_RETURN: usize = 0;
pub const K_TERM_CHANGE_SCROLL_REGION: usize = 1;
pub const K_TERM_CLEAR_SCREEN: usize = 2;
pub const K_TERM_CLR_EOL: usize = 3;
pub const K_TERM_CLR_EOS: usize = 4;
pub const K_TERM_CURSOR_ADDRESS: usize = 5;
pub const K_TERM_CURSOR_DOWN: usize = 6;
pub const K_TERM_CURSOR_INVISIBLE: usize = 7;
pub const K_TERM_CURSOR_LEFT: usize = 8;
pub const K_TERM_CURSOR_HOME: usize = 9;
pub const K_TERM_CURSOR_NORMAL: usize = 10;
pub const K_TERM_CURSOR_UP: usize = 11;
pub const K_TERM_CURSOR_RIGHT: usize = 12;
pub const K_TERM_DELETE_LINE: usize = 13;
pub const K_TERM_ENTER_BLINK_MODE: usize = 14;
pub const K_TERM_ENTER_BOLD_MODE: usize = 15;
pub const K_TERM_ENTER_CA_MODE: usize = 16;
pub const K_TERM_ENTER_DIM_MODE: usize = 17;
pub const K_TERM_ENTER_ITALICS_MODE: usize = 18;
pub const K_TERM_ENTER_REVERSE_MODE: usize = 19;
pub const K_TERM_ENTER_SECURE_MODE: usize = 20;
pub const K_TERM_ENTER_STANDOUT_MODE: usize = 21;
pub const K_TERM_ENTER_UNDERLINE_MODE: usize = 22;
pub const K_TERM_ERASE_CHARS: usize = 23;
pub const K_TERM_EXIT_ATTRIBUTE_MODE: usize = 24;
pub const K_TERM_EXIT_CA_MODE: usize = 25;
pub const K_TERM_FROM_STATUS_LINE: usize = 26;
pub const K_TERM_INSERT_LINE: usize = 27;
pub const K_TERM_KEYPAD_LOCAL: usize = 28;
pub const K_TERM_KEYPAD_XMIT: usize = 29;
pub const K_TERM_PARM_DELETE_LINE: usize = 30;
pub const K_TERM_PARM_DOWN_CURSOR: usize = 31;
pub const K_TERM_PARM_INSERT_LINE: usize = 32;
pub const K_TERM_PARM_LEFT_CURSOR: usize = 33;
pub const K_TERM_PARM_RIGHT_CURSOR: usize = 34;
pub const K_TERM_PARM_UP_CURSOR: usize = 35;
pub const K_TERM_SET_A_BACKGROUND: usize = 36;
pub const K_TERM_SET_A_FOREGROUND: usize = 37;
pub const K_TERM_SET_ATTRIBUTES: usize = 38;
pub const K_TERM_SET_LR_MARGIN: usize = 39;
pub const K_TERM_TO_STATUS_LINE: usize = 40;
pub const K_TERM_EXT_OFFSET: usize = 41;
pub const K_TERM_RESET_CURSOR_STYLE: usize = 41;
pub const K_TERM_SET_CURSOR_STYLE: usize = 42;
pub const K_TERM_ENTER_STRIKETHROUGH_MODE: usize = 43;
pub const K_TERM_SET_RGB_FOREGROUND: usize = 44;
pub const K_TERM_SET_RGB_BACKGROUND: usize = 45;
pub const K_TERM_SET_CURSOR_COLOR: usize = 46;
pub const K_TERM_RESET_CURSOR_COLOR: usize = 47;
pub const K_TERM_SET_UNDERLINE_STYLE: usize = 48;

pub const K_TERM_KEY_BACKSPACE: usize = 0;
pub const K_TERM_KEY_BEG: usize = 1;
pub const K_TERM_KEY_BTAB: usize = 2;
pub const K_TERM_KEY_CLEAR: usize = 3;
pub const K_TERM_KEY_DC: usize = 4;
pub const K_TERM_KEY_END: usize = 5;
pub const K_TERM_KEY_FIND: usize = 6;
pub const K_TERM_KEY_HOME: usize = 7;
pub const K_TERM_KEY_IC: usize = 8;
pub const K_TERM_KEY_NPAGE: usize = 9;
pub const K_TERM_KEY_PPAGE: usize = 10;
pub const K_TERM_KEY_SELECT: usize = 11;
pub const K_TERM_KEY_SUSPEND: usize = 12;
pub const K_TERM_KEY_UNDO: usize = 13;
pub const K_TERM_KEY_LEFT: usize = 14;
pub const K_TERM_KEY_RIGHT: usize = 15;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TPVAR {
    pub num: c_long,
    pub string: Option<[c_char; 8]>,
}

#[repr(C)]
struct TPSTACK {
    nums: [c_long; 20],
    strings: [Option<[c_char; 8]>; 20],
    offset: usize,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnibiBoolean {
    UnibiBooleanBeginMarker = 0,
    UnibiAutoLeftMargin,
    UnibiAutoRightMargin,
    UnibiNoEscCtlc,
    UnibiCeolStandoutGlitch,
    UnibiEatNewlineGlitch,
    UnibiEraseOverstrike,
    UnibiGenericType,
    UnibiHardCopy,
    UnibiHasMetaKey,
    UnibiHasStatusLine,
    UnibiInsertNullGlitch,
    UnibiMemoryAbove,
    UnibiMemoryBelow,
    UnibiMoveInsertMode,
    UnibiMoveStandoutMode,
    UnibiOverStrike,
    UnibiStatusLineEscOk,
    UnibiDestTabsMagicSmso,
    UnibiTildeGlitch,
    UnibiTransparentUnderline,
    UnibiXonXoff,
    UnibiNeedsXonXoff,
    UnibiPrtrSilent,
    UnibiHardCursor,
    UnibiNonRevRmcup,
    UnibiNoPadChar,
    UnibiNonDestScrollRegion,
    UnibiCanChange,
    UnibiBackColorErase,
    UnibiHueLightnessSaturation,
    UnibiColAddrGlitch,
    UnibiCrCancelsMicroMode,
    UnibiHasPrintWheel,
    UnibiRowAddrGlitch,
    UnibiSemiAutoRightMargin,
    UnibiCpiChangesRes,
    UnibiLpiChangesRes,
    UnibiBackspacesWithBs,
    UnibiCrtNoScrolling,
    UnibiNoCorrectlyWorkingCr,
    UnibiGnuHasMetaKey,
    UnibiLinefeedIsNewline,
    UnibiHasHardwareTabs,
    UnibiReturnDoesClrEol,
    UnibiBooleanEndMarker,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnibiNumeric {
    UnibiNumericBeginMarker = UnibiBoolean::UnibiBooleanEndMarker as isize,
    UnibiColumns,
    UnibiInitTabs,
    UnibiLines,
    UnibiLinesOfMemory,
    UnibiMagicCookieGlitch,
    UnibiPaddingBaudRate,
    UnibiVirtualTerminal,
    UnibiWidthStatusLine,
    UnibiNumLabels,
    UnibiLabelHeight,
    UnibiLabelWidth,
    UnibiMaxAttributes,
    UnibiMaximumWindows,
    UnibiMaxColors,
    UnibiMaxPairs,
    UnibiNoColorVideo,
    UnibiBufferCapacity,
    UnibiDotVertSpacing,
    UnibiDotHorzSpacing,
    UnibiMaxMicroAddress,
    UnibiMaxMicroJump,
    UnibiMicroColSize,
    UnibiMicroLineSize,
    UnibiNumberOfPins,
    UnibiOutputResChar,
    UnibiOutputResLine,
    UnibiOutputResHorzInch,
    UnibiOutputResVertInch,
    UnibiPrintRate,
    UnibiWideCharSize,
    UnibiButtons,
    UnibiBitImageEntwining,
    UnibiBitImageType,
    UnibiMagicCookieGlitchUl,
    UnibiCarriageReturnDelay,
    UnibiNewLineDelay,
    UnibiBackspaceDelay,
    UnibiHorizontalTabDelay,
    UnibiNumberOfFunctionKeys,
    UnibiNumericEndMarker,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnibiString {
    UnibiStringBeginMarker = UnibiNumeric::UnibiNumericEndMarker as isize,
    UnibiBackTab,
    UnibiBell,
    UnibiCarriageReturn,
    UnibiChangeScrollRegion,
    UnibiClearAllTabs,
    UnibiClearScreen,
    UnibiClrEol,
    UnibiClrEos,
    UnibiColumnAddress,
    UnibiCommandCharacter,
    UnibiCursorAddress,
    UnibiCursorDown,
    UnibiCursorHome,
    UnibiCursorInvisible,
    UnibiCursorLeft,
    UnibiCursorMemAddress,
    UnibiCursorNormal,
    UnibiCursorRight,
    UnibiCursorToLl,
    UnibiCursorUp,
    UnibiCursorVisible,
    UnibiDeleteCharacter,
    UnibiDeleteLine,
    UnibiDisStatusLine,
    UnibiDownHalfLine,
    UnibiEnterAltCharsetMode,
    UnibiEnterBlinkMode,
    UnibiEnterBoldMode,
    UnibiEnterCaMode,
    UnibiEnterDeleteMode,
    UnibiEnterDimMode,
    UnibiEnterInsertMode,
    UnibiEnterSecureMode,
    UnibiEnterProtectedMode,
    UnibiEnterReverseMode,
    UnibiEnterStandoutMode,
    UnibiEnterUnderlineMode,
    UnibiEraseChars,
    UnibiExitAltCharsetMode,
    UnibiExitAttributeMode,
    UnibiExitCaMode,
    UnibiExitDeleteMode,
    UnibiExitInsertMode,
    UnibiExitStandoutMode,
    UnibiExitUnderlineMode,
    UnibiFlashScreen,
    UnibiFormFeed,
    UnibiFromStatusLine,
    UnibiInit1string,
    UnibiInit2string,
    UnibiInit3string,
    UnibiInitFile,
    UnibiInsertCharacter,
    UnibiInsertLine,
    UnibiInsertPadding,
    UnibiKeyBackspace,
    UnibiKeyCatab,
    UnibiKeyClear,
    UnibiKeyCtab,
    UnibiKeyDc,
    UnibiKeyDl,
    UnibiKeyDown,
    UnibiKeyEic,
    UnibiKeyEol,
    UnibiKeyEos,
    UnibiKeyF0,
    UnibiKeyF1,
    UnibiKeyF10,
    UnibiKeyF2,
    UnibiKeyF3,
    UnibiKeyF4,
    UnibiKeyF5,
    UnibiKeyF6,
    UnibiKeyF7,
    UnibiKeyF8,
    UnibiKeyF9,
    UnibiKeyHome,
    UnibiKeyIc,
    UnibiKeyIl,
    UnibiKeyLeft,
    UnibiKeyLl,
    UnibiKeyNpage,
    UnibiKeyPpage,
    UnibiKeyRight,
    UnibiKeySf,
    UnibiKeySr,
    UnibiKeyStab,
    UnibiKeyUp,
    UnibiKeypadLocal,
    UnibiKeypadXmit,
    UnibiLabF0,
    UnibiLabF1,
    UnibiLabF10,
    UnibiLabF2,
    UnibiLabF3,
    UnibiLabF4,
    UnibiLabF5,
    UnibiLabF6,
    UnibiLabF7,
    UnibiLabF8,
    UnibiLabF9,
    UnibiMetaOff,
    UnibiMetaOn,
    UnibiNewline,
    UnibiPadChar,
    UnibiParmDch,
    UnibiParmDeleteLine,
    UnibiParmDownCursor,
    UnibiParmIch,
    UnibiParmIndex,
    UnibiParmInsertLine,
    UnibiParmLeftCursor,
    UnibiParmRightCursor,
    UnibiParmRindex,
    UnibiParmUpCursor,
    UnibiPkeyKey,
    UnibiPkeyLocal,
    UnibiPkeyXmit,
    UnibiPrintScreen,
    UnibiPrtrOff,
    UnibiPrtrOn,
    UnibiRepeatChar,
    UnibiReset1string,
    UnibiReset2string,
    UnibiReset3string,
    UnibiResetFile,
    UnibiRestoreCursor,
    UnibiRowAddress,
    UnibiSaveCursor,
    UnibiScrollForward,
    UnibiScrollReverse,
    UnibiSetAttributes,
    UnibiSetTab,
    UnibiSetWindow,
    UnibiTab,
    UnibiToStatusLine,
    UnibiUnderlineChar,
    UnibiUpHalfLine,
    UnibiInitProg,
    UnibiKeyA1,
    UnibiKeyA3,
    UnibiKeyB2,
    UnibiKeyC1,
    UnibiKeyC3,
    UnibiPrtrNon,
    UnibiCharPadding,
    UnibiAcsChars,
    UnibiPlabNorm,
    UnibiKeyBtab,
    UnibiEnterXonMode,
    UnibiExitXonMode,
    UnibiEnterAmMode,
    UnibiExitAmMode,
    UnibiXonCharacter,
    UnibiXoffCharacter,
    UnibiEnaAcs,
    UnibiLabelOn,
    UnibiLabelOff,
    UnibiKeyBeg,
    UnibiKeyCancel,
    UnibiKeyClose,
    UnibiKeyCommand,
    UnibiKeyCopy,
    UnibiKeyCreate,
    UnibiKeyEnd,
    UnibiKeyEnter,
    UnibiKeyExit,
    UnibiKeyFind,
    UnibiKeyHelp,
    UnibiKeyMark,
    UnibiKeyMessage,
    UnibiKeyMove,
    UnibiKeyNext,
    UnibiKeyOpen,
    UnibiKeyOptions,
    UnibiKeyPrevious,
    UnibiKeyPrint,
    UnibiKeyRedo,
    UnibiKeyReference,
    UnibiKeyRefresh,
    UnibiKeyReplace,
    UnibiKeyRestart,
    UnibiKeyResume,
    UnibiKeySave,
    UnibiKeySuspend,
    UnibiKeyUndo,
    UnibiKeySbeg,
    UnibiKeyScancel,
    UnibiKeyScommand,
    UnibiKeyScopy,
    UnibiKeyScreate,
    UnibiKeySdc,
    UnibiKeySdl,
    UnibiKeySelect,
    UnibiKeySend,
    UnibiKeySeol,
    UnibiKeySexit,
    UnibiKeySfind,
    UnibiKeyShelp,
    UnibiKeyShome,
    UnibiKeySic,
    UnibiKeySleft,
    UnibiKeySmessage,
    UnibiKeySmove,
    UnibiKeySnext,
    UnibiKeySoptions,
    UnibiKeySprevious,
    UnibiKeySprint,
    UnibiKeySredo,
    UnibiKeySreplace,
    UnibiKeySright,
    UnibiKeySrsume,
    UnibiKeySsave,
    UnibiKeySsuspend,
    UnibiKeySundo,
    UnibiReqForInput,
    UnibiKeyF11,
    UnibiKeyF12,
    UnibiKeyF13,
    UnibiKeyF14,
    UnibiKeyF15,
    UnibiKeyF16,
    UnibiKeyF17,
    UnibiKeyF18,
    UnibiKeyF19,
    UnibiKeyF20,
    UnibiKeyF21,
    UnibiKeyF22,
    UnibiKeyF23,
    UnibiKeyF24,
    UnibiKeyF25,
    UnibiKeyF26,
    UnibiKeyF27,
    UnibiKeyF28,
    UnibiKeyF29,
    UnibiKeyF30,
    UnibiKeyF31,
    UnibiKeyF32,
    UnibiKeyF33,
    UnibiKeyF34,
    UnibiKeyF35,
    UnibiKeyF36,
    UnibiKeyF37,
    UnibiKeyF38,
    UnibiKeyF39,
    UnibiKeyF40,
    UnibiKeyF41,
    UnibiKeyF42,
    UnibiKeyF43,
    UnibiKeyF44,
    UnibiKeyF45,
    UnibiKeyF46,
    UnibiKeyF47,
    UnibiKeyF48,
    UnibiKeyF49,
    UnibiKeyF50,
    UnibiKeyF51,
    UnibiKeyF52,
    UnibiKeyF53,
    UnibiKeyF54,
    UnibiKeyF55,
    UnibiKeyF56,
    UnibiKeyF57,
    UnibiKeyF58,
    UnibiKeyF59,
    UnibiKeyF60,
    UnibiKeyF61,
    UnibiKeyF62,
    UnibiKeyF63,
    UnibiClrBol,
    UnibiClearMargins,
    UnibiSetLeftMargin,
    UnibiSetRightMargin,
    UnibiLabelFormat,
    UnibiSetClock,
    UnibiDisplayClock,
    UnibiRemoveClock,
    UnibiCreateWindow,
    UnibiGotoWindow,
    UnibiHangup,
    UnibiDialPhone,
    UnibiQuickDial,
    UnibiTone,
    UnibiPulse,
    UnibiFlashHook,
    UnibiFixedPause,
    UnibiWaitTone,
    UnibiUser0,
    UnibiUser1,
    UnibiUser2,
    UnibiUser3,
    UnibiUser4,
    UnibiUser5,
    UnibiUser6,
    UnibiUser7,
    UnibiUser8,
    UnibiUser9,
    UnibiOrigPair,
    UnibiOrigColors,
    UnibiInitializeColor,
    UnibiInitializePair,
    UnibiSetColorPair,
    UnibiSetForeground,
    UnibiSetBackground,
    UnibiChangeCharPitch,
    UnibiChangeLinePitch,
    UnibiChangeResHorz,
    UnibiChangeResVert,
    UnibiDefineChar,
    UnibiEnterDoublewideMode,
    UnibiEnterDraftQuality,
    UnibiEnterItalicsMode,
    UnibiEnterLeftwardMode,
    UnibiEnterMicroMode,
    UnibiEnterNearLetterQuality,
    UnibiEnterNormalQuality,
    UnibiEnterShadowMode,
    UnibiEnterSubscriptMode,
    UnibiEnterSuperscriptMode,
    UnibiEnterUpwardMode,
    UnibiExitDoublewideMode,
    UnibiExitItalicsMode,
    UnibiExitLeftwardMode,
    UnibiExitMicroMode,
    UnibiExitShadowMode,
    UnibiExitSubscriptMode,
    UnibiExitSuperscriptMode,
    UnibiExitUpwardMode,
    UnibiMicroColumnAddress,
    UnibiMicroDown,
    UnibiMicroLeft,
    UnibiMicroRight,
    UnibiMicroRowAddress,
    UnibiMicroUp,
    UnibiOrderOfPins,
    UnibiParmDownMicro,
    UnibiParmLeftMicro,
    UnibiParmRightMicro,
    UnibiParmUpMicro,
    UnibiSelectCharSet,
    UnibiSetBottomMargin,
    UnibiSetBottomMarginParm,
    UnibiSetLeftMarginParm,
    UnibiSetRightMarginParm,
    UnibiSetTopMargin,
    UnibiSetTopMarginParm,
    UnibiStartBitImage,
    UnibiStartCharSetDef,
    UnibiStopBitImage,
    UnibiStopCharSetDef,
    UnibiSubscriptCharacters,
    UnibiSuperscriptCharacters,
    UnibiTheseCauseCr,
    UnibiZeroMotion,
    UnibiCharSetNames,
    UnibiKeyMouse,
    UnibiMouseInfo,
    UnibiReqMousePos,
    UnibiGetMouse,
    UnibiSetAForeground,
    UnibiSetABackground,
    UnibiPkeyPlab,
    UnibiDeviceType,
    UnibiCodeSetInit,
    UnibiSet0DesSeq,
    UnibiSet1DesSeq,
    UnibiSet2DesSeq,
    UnibiSet3DesSeq,
    UnibiSetLrMargin,
    UnibiSetTbMargin,
    UnibiBitImageRepeat,
    UnibiBitImageNewline,
    UnibiBitImageCarriageReturn,
    UnibiColorNames,
    UnibiDefineBitImageRegion,
    UnibiEndBitImageRegion,
    UnibiSetColorBand,
    UnibiSetPageLength,
    UnibiDisplayPcChar,
    UnibiEnterPcCharsetMode,
    UnibiExitPcCharsetMode,
    UnibiEnterScancodeMode,
    UnibiExitScancodeMode,
    UnibiPcTermOptions,
    UnibiScancodeEscape,
    UnibiAltScancodeEsc,
    UnibiEnterHorizontalHlMode,
    UnibiEnterLeftHlMode,
    UnibiEnterLowHlMode,
    UnibiEnterRightHlMode,
    UnibiEnterTopHlMode,
    UnibiEnterVerticalHlMode,
    UnibiSetAAttributes,
    UnibiSetPglenInch,
    UnibiTermcapInit2,
    UnibiTermcapReset,
    UnibiLinefeedIfNotLf,
    UnibiBackspaceIfNotBs,
    UnibiOtherNonFunctionKeys,
    UnibiArrowKeyMap,
    UnibiAcsUlcorner,
    UnibiAcsLlcorner,
    UnibiAcsUrcorner,
    UnibiAcsLrcorner,
    UnibiAcsLtee,
    UnibiAcsRtee,
    UnibiAcsBtee,
    UnibiAcsTtee,
    UnibiAcsHline,
    UnibiAcsVline,
    UnibiAcsPlus,
    UnibiMemoryLock,
    UnibiMemoryUnlock,
    UnibiBoxChars1,
    UnibiStringEndMarker,
}

#[derive(Clone, Copy)]
struct BuiltinTerminfoEntry {
    back_color_erase: bool,
    Tc: bool,
    RGB: bool,
    Su: bool,
    max_colors: c_int,
    lines: c_int,
    columns: c_int,
    defs: [Option<&'static CStr>; K_TERM_COUNT],
    keys: [[Option<&'static CStr>; 2]; K_TERM_KEY_COUNT],
    f_keys: [Option<&'static CStr>; K_TERMINFO_FUNC_KEY_MAX],
}

impl BuiltinTerminfoEntry {
    fn to_entry(self) -> TerminfoEntry {
        TerminfoEntry {
            back_color_erase: self.back_color_erase,
            Tc: self.Tc,
            RGB: self.RGB,
            Su: self.Su,
            max_colors: self.max_colors,
            lines: self.lines,
            columns: self.columns,
            defs: self.defs.map(|opt| opt.map(std::borrow::Cow::Borrowed)),
            keys: self
                .keys
                .map(|pair| pair.map(|opt| opt.map(std::borrow::Cow::Borrowed))),
            f_keys: self.f_keys.map(|opt| opt.map(std::borrow::Cow::Borrowed)),
        }
    }
}

impl TerminfoEntry {
    pub const fn new() -> Self {
        Self {
            back_color_erase: false,
            Tc: false,
            RGB: false,
            Su: false,
            max_colors: 0,
            lines: 0,
            columns: 0,
            defs: [const { None }; K_TERM_COUNT],
            keys: [const { [None, None] }; K_TERM_KEY_COUNT],
            f_keys: [const { None }; K_TERMINFO_FUNC_KEY_MAX],
        }
    }
}

const fn empty_entry(
    back_color_erase: bool,
    max_colors: c_int,
    lines: c_int,
    columns: c_int,
) -> BuiltinTerminfoEntry {
    BuiltinTerminfoEntry {
        back_color_erase,
        Tc: false,
        RGB: false,
        Su: false,
        max_colors,
        lines,
        columns,
        defs: [None; K_TERM_COUNT],
        keys: [[None; 2]; K_TERM_KEY_COUNT],
        f_keys: [None; K_TERMINFO_FUNC_KEY_MAX],
    }
}

static ANSI_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(false, 8, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\x1b[B");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x1b[D");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BLINK_MODE] = Some(c"\x1b[5m");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_SECURE_MODE] = Some(c"\x1b[8m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_ERASE_CHARS] = Some(c"\x1b[%p1%dX");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[0;10m");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] = Some(c"\x1b[4%p1%dm");
    e.defs[K_TERM_SET_A_FOREGROUND] = Some(c"\x1b[3%p1%dm");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"\x1b[0;10%?%p1%t;7%;%?%p2%t;4%;%?%p3%t;7%;%?%p4%t;5%;%?%p6%t;1%;%?%p7%t;8%;%?%p9%t;11%;m");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x08"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[H"), None];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[L"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1b[D"), None];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1b[C"), None];
    e
};

static GHOSTTY_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[2J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\n");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[?12l\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[?1049h");
    e.defs[K_TERM_ENTER_DIM_MODE] = Some(c"\x1b[2m");
    e.defs[K_TERM_ENTER_ITALICS_MODE] = Some(c"\x1b[3m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_SECURE_MODE] = Some(c"\x1b[8m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_ERASE_CHARS] = Some(c"\x1b[%p1%dX");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b(B\x1b[m");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[?1049l");
    e.defs[K_TERM_FROM_STATUS_LINE] = Some(c"\x07");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_KEYPAD_LOCAL] = Some(c"\x1b[?1l\x1b>");
    e.defs[K_TERM_KEYPAD_XMIT] = Some(c"\x1b[?1h\x1b=");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[K_TERM_SET_A_FOREGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"%?%p9%t\x1b(0%e\x1b(B%;\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p5%t;2%;%?%p7%t;8%;m");
    e.defs[K_TERM_SET_LR_MARGIN] = Some(c"\x1b[?69h\x1b[%i%p1%d;%p2%ds");
    e.defs[K_TERM_TO_STATUS_LINE] = Some(c"\x1b]2;");
    e.defs[K_TERM_RESET_CURSOR_STYLE] = Some(c"\x1b[2 q");
    e.defs[K_TERM_SET_CURSOR_STYLE] = Some(c"\x1b[%p1%d q");
    e.defs[K_TERM_ENTER_STRIKETHROUGH_MODE] = Some(c"\x1b[9m");
    e.defs[K_TERM_SET_UNDERLINE_STYLE] = Some(c"\x1b[4:%p1%dm");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x7f"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1bOF"), Some(c"\x1b[1;2F")];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1bOH"), Some(c"\x1b[1;2H")];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
    e.f_keys[0] = Some(c"\x1bOP");
    e.f_keys[1] = Some(c"\x1bOQ");
    e.f_keys[2] = Some(c"\x1bOR");
    e.f_keys[3] = Some(c"\x1bOS");
    e.f_keys[4] = Some(c"\x1b[15~");
    e.f_keys[5] = Some(c"\x1b[17~");
    e.f_keys[6] = Some(c"\x1b[18~");
    e.f_keys[7] = Some(c"\x1b[19~");
    e.f_keys[8] = Some(c"\x1b[20~");
    e.f_keys[9] = Some(c"\x1b[21~");
    e.f_keys[10] = Some(c"\x1b[23~");
    e.f_keys[11] = Some(c"\x1b[24~");
    e.f_keys[12] = Some(c"\x1b[1;2P");
    e.f_keys[13] = Some(c"\x1b[1;2Q");
    e.f_keys[14] = Some(c"\x1b[1;2R");
    e.f_keys[15] = Some(c"\x1b[1;2S");
    e.f_keys[16] = Some(c"\x1b[15;2~");
    e.f_keys[17] = Some(c"\x1b[17;2~");
    e.f_keys[18] = Some(c"\x1b[18;2~");
    e.f_keys[19] = Some(c"\x1b[19;2~");
    e.f_keys[20] = Some(c"\x1b[20;2~");
    e.f_keys[21] = Some(c"\x1b[21;2~");
    e.f_keys[22] = Some(c"\x1b[23;2~");
    e.f_keys[23] = Some(c"\x1b[24;2~");
    e.f_keys[24] = Some(c"\x1b[1;5P");
    e.f_keys[25] = Some(c"\x1b[1;5Q");
    e.f_keys[26] = Some(c"\x1b[1;5R");
    e.f_keys[27] = Some(c"\x1b[1;5S");
    e.f_keys[28] = Some(c"\x1b[15;5~");
    e.f_keys[29] = Some(c"\x1b[17;5~");
    e.f_keys[30] = Some(c"\x1b[18;5~");
    e.f_keys[31] = Some(c"\x1b[19;5~");
    e.f_keys[32] = Some(c"\x1b[20;5~");
    e.f_keys[33] = Some(c"\x1b[21;5~");
    e.f_keys[34] = Some(c"\x1b[23;5~");
    e.f_keys[35] = Some(c"\x1b[24;5~");
    e.f_keys[36] = Some(c"\x1b[1;6P");
    e.f_keys[37] = Some(c"\x1b[1;6Q");
    e.f_keys[38] = Some(c"\x1b[1;6R");
    e.f_keys[39] = Some(c"\x1b[1;6S");
    e.f_keys[40] = Some(c"\x1b[15;6~");
    e.f_keys[41] = Some(c"\x1b[17;6~");
    e.f_keys[42] = Some(c"\x1b[18;6~");
    e.f_keys[43] = Some(c"\x1b[19;6~");
    e.f_keys[44] = Some(c"\x1b[20;6~");
    e.f_keys[45] = Some(c"\x1b[21;6~");
    e.f_keys[46] = Some(c"\x1b[23;6~");
    e.f_keys[47] = Some(c"\x1b[24;6~");
    e.f_keys[48] = Some(c"\x1b[1;3P");
    e.f_keys[49] = Some(c"\x1b[1;3Q");
    e.f_keys[50] = Some(c"\x1b[1;3R");
    e.f_keys[51] = Some(c"\x1b[1;3S");
    e.f_keys[52] = Some(c"\x1b[15;3~");
    e.f_keys[53] = Some(c"\x1b[17;3~");
    e.f_keys[54] = Some(c"\x1b[18;3~");
    e.f_keys[55] = Some(c"\x1b[19;3~");
    e.f_keys[56] = Some(c"\x1b[20;3~");
    e.f_keys[57] = Some(c"\x1b[21;3~");
    e.f_keys[58] = Some(c"\x1b[23;3~");
    e.f_keys[59] = Some(c"\x1b[24;3~");
    e.f_keys[60] = Some(c"\x1b[1;4P");
    e.f_keys[61] = Some(c"\x1b[1;4Q");
    e.f_keys[62] = Some(c"\x1b[1;4R");
    e
};

static INTERIX_8COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 8, 25, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[2J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\n");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x1b[D");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[s\x1b[1b");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[0m");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[2b\x1b[u\r\x1b[K");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] = Some(c"\x1b[4%p1%dm");
    e.defs[K_TERM_SET_A_FOREGROUND] = Some(c"\x1b[3%p1%dm");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x08"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x7f"), None];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[U"), None];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[H"), None];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[L"), None];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[T"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[S"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1b[D"), Some(c"\x1bF^")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1b[C"), Some(c"\x1bF$")];
    e.f_keys[0] = Some(c"\x1bF1");
    e.f_keys[1] = Some(c"\x1bF2");
    e.f_keys[2] = Some(c"\x1bF3");
    e.f_keys[3] = Some(c"\x1bF4");
    e.f_keys[4] = Some(c"\x1bF5");
    e.f_keys[5] = Some(c"\x1bF6");
    e.f_keys[6] = Some(c"\x1bF7");
    e.f_keys[7] = Some(c"\x1bF8");
    e.f_keys[8] = Some(c"\x1bF9");
    e.f_keys[9] = Some(c"\x1bFA");
    e.f_keys[10] = Some(c"\x1bFB");
    e.f_keys[11] = Some(c"\x1bFC");
    e.f_keys[12] = Some(c"\x1bFD");
    e.f_keys[13] = Some(c"\x1bFE");
    e.f_keys[14] = Some(c"\x1bFF");
    e.f_keys[15] = Some(c"\x1bFG");
    e.f_keys[16] = Some(c"\x1bFH");
    e.f_keys[17] = Some(c"\x1bFI");
    e.f_keys[18] = Some(c"\x1bFJ");
    e.f_keys[19] = Some(c"\x1bFK");
    e.f_keys[20] = Some(c"\x1bFL");
    e.f_keys[21] = Some(c"\x1bFM");
    e.f_keys[22] = Some(c"\x1bFN");
    e.f_keys[23] = Some(c"\x1bFO");
    e.f_keys[24] = Some(c"\x1bFP");
    e.f_keys[25] = Some(c"\x1bFQ");
    e.f_keys[26] = Some(c"\x1bFR");
    e.f_keys[27] = Some(c"\x1bFS");
    e.f_keys[28] = Some(c"\x1bFT");
    e.f_keys[29] = Some(c"\x1bFU");
    e.f_keys[30] = Some(c"\x1bFV");
    e.f_keys[31] = Some(c"\x1bFW");
    e.f_keys[32] = Some(c"\x1bFX");
    e.f_keys[33] = Some(c"\x1bFY");
    e.f_keys[34] = Some(c"\x1bFZ");
    e.f_keys[35] = Some(c"\x1bFa");
    e.f_keys[36] = Some(c"\x1bFb");
    e.f_keys[37] = Some(c"\x1bFc");
    e.f_keys[38] = Some(c"\x1bFd");
    e.f_keys[39] = Some(c"\x1bFe");
    e.f_keys[40] = Some(c"\x1bFf");
    e.f_keys[41] = Some(c"\x1bFg");
    e.f_keys[42] = Some(c"\x1bFh");
    e.f_keys[43] = Some(c"\x1bFi");
    e.f_keys[44] = Some(c"\x1bFj");
    e.f_keys[45] = Some(c"\x1bFk");
    e.f_keys[46] = Some(c"\x1bFm");
    e.f_keys[47] = Some(c"\x1bFn");
    e.f_keys[48] = Some(c"\x1bFo");
    e.f_keys[49] = Some(c"\x1bFp");
    e.f_keys[50] = Some(c"\x1bFq");
    e.f_keys[51] = Some(c"\x1bFr");
    e.f_keys[52] = Some(c"\x1bFs");
    e.f_keys[53] = Some(c"\x1bFt");
    e.f_keys[54] = Some(c"\x1bFu");
    e.f_keys[55] = Some(c"\x1bFv");
    e.f_keys[56] = Some(c"\x1bFw");
    e.f_keys[57] = Some(c"\x1bFx");
    e.f_keys[58] = Some(c"\x1bFy");
    e.f_keys[59] = Some(c"\x1bFz");
    e
};

static ITERM_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\n");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BLINK_MODE] = Some(c"\x1b[5m");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[?1049h\x1b[22;0;0t");
    e.defs[K_TERM_ENTER_DIM_MODE] = Some(c"\x1b[2m");
    e.defs[K_TERM_ENTER_ITALICS_MODE] = Some(c"\x1b[3m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[m\x0f");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[?1049l\x1b[23;0;0t");
    e.defs[K_TERM_FROM_STATUS_LINE] = Some(c"\x07");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_KEYPAD_LOCAL] = Some(c"\x1b[?1l\x1b>");
    e.defs[K_TERM_KEYPAD_XMIT] = Some(c"\x1b[?1h\x1b=");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[K_TERM_SET_A_FOREGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;%?%p5%t;2%;m%?%p9%t^N%e\x0f%;");
    e.defs[K_TERM_TO_STATUS_LINE] = Some(c"\x1b]2;");
    e.defs[K_TERM_ENTER_STRIKETHROUGH_MODE] = Some(c"\x1b[9m");
    e.defs[K_TERM_SET_UNDERLINE_STYLE] = Some(c"\x1b[4:%p1%dm");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x7f"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), None];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1bOF"), Some(c"\x1b[1;2F")];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1bOH"), Some(c"\x1b[1;2H")];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
    e.f_keys[0] = Some(c"\x1bOP");
    e.f_keys[1] = Some(c"\x1bOQ");
    e.f_keys[2] = Some(c"\x1bOR");
    e.f_keys[3] = Some(c"\x1bOS");
    e.f_keys[4] = Some(c"\x1b[15~");
    e.f_keys[5] = Some(c"\x1b[17~");
    e.f_keys[6] = Some(c"\x1b[18~");
    e.f_keys[7] = Some(c"\x1b[19~");
    e.f_keys[8] = Some(c"\x1b[20~");
    e.f_keys[9] = Some(c"\x1b[21~");
    e.f_keys[10] = Some(c"\x1b[23~");
    e.f_keys[11] = Some(c"\x1b[24~");
    e.f_keys[12] = Some(c"\x1b[1;2P");
    e.f_keys[13] = Some(c"\x1b[1;2Q");
    e.f_keys[14] = Some(c"\x1b[1;2R");
    e.f_keys[15] = Some(c"\x1b[1;2S");
    e.f_keys[16] = Some(c"\x1b[15;2~");
    e.f_keys[17] = Some(c"\x1b[17;2~");
    e.f_keys[18] = Some(c"\x1b[18;2~");
    e.f_keys[19] = Some(c"\x1b[19;2~");
    e.f_keys[20] = Some(c"\x1b[20;2~");
    e.f_keys[21] = Some(c"\x1b[21;2~");
    e.f_keys[22] = Some(c"\x1b[23;2~");
    e.f_keys[23] = Some(c"\x1b[24;2~");
    e
};

static LINUX_16COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 8, -1, -1);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\n");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l\x1b[?1c");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[?25h\x1b[?0c");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BLINK_MODE] = Some(c"\x1b[5m");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_DIM_MODE] = Some(c"\x1b[2m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_ERASE_CHARS] = Some(c"\x1b[%p1%dX");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[m\x0f");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] = Some(c"\x1b[4%p1%dm");
    e.defs[K_TERM_SET_A_FOREGROUND] = Some(c"\x1b[3%p1%dm");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"\x1b[0;10%?%p1%t;7%;%?%p2%t;4%;%?%p3%t;7%;%?%p4%t;5%;%?%p5%t;2%;%?%p6%t;1%;m%?%p9%t^N%e\x0f%;");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x7f"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b^I"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), None];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[4~"), None];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[1~"), None];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), None];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_SUSPEND] = [Some(c"^Z"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1b[D"), None];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1b[C"), None];
    e.f_keys[0] = Some(c"\x1b[[A");
    e.f_keys[1] = Some(c"\x1b[[B");
    e.f_keys[2] = Some(c"\x1b[[C");
    e.f_keys[3] = Some(c"\x1b[[D");
    e.f_keys[4] = Some(c"\x1b[[E");
    e.f_keys[5] = Some(c"\x1b[17~");
    e.f_keys[6] = Some(c"\x1b[18~");
    e.f_keys[7] = Some(c"\x1b[19~");
    e.f_keys[8] = Some(c"\x1b[20~");
    e.f_keys[9] = Some(c"\x1b[21~");
    e.f_keys[10] = Some(c"\x1b[23~");
    e.f_keys[11] = Some(c"\x1b[24~");
    e.f_keys[12] = Some(c"\x1b[25~");
    e.f_keys[13] = Some(c"\x1b[26~");
    e.f_keys[14] = Some(c"\x1b[28~");
    e.f_keys[15] = Some(c"\x1b[29~");
    e.f_keys[16] = Some(c"\x1b[31~");
    e.f_keys[17] = Some(c"\x1b[32~");
    e.f_keys[18] = Some(c"\x1b[33~");
    e.f_keys[19] = Some(c"\x1b[34~");
    e
};

static PUTTY_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, -1, -1);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\x1bD");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1bM");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BLINK_MODE] = Some(c"\x1b[5m");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[?1049h");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_ERASE_CHARS] = Some(c"\x1b[%p1%dX");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[m\x0f");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[?1049l");
    e.defs[K_TERM_FROM_STATUS_LINE] = Some(c"\x07");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_KEYPAD_LOCAL] = Some(c"\x1b[?1l\x1b>");
    e.defs[K_TERM_KEYPAD_XMIT] = Some(c"\x1b[?1h\x1b=");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[K_TERM_SET_A_FOREGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"\x1b[0%?%p1%p6%|%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;m%?%p9%t^N%e\x0f%;");
    e.defs[K_TERM_TO_STATUS_LINE] = Some(c"\x1b]0;");
    e.defs[K_TERM_ENTER_STRIKETHROUGH_MODE] = Some(c"\x1b[9m");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x7f"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), None];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[4~"), None];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[1~"), None];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), None];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_SUSPEND] = [Some(c"^Z"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1bOD"), None];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1bOC"), None];
    e.f_keys[0] = Some(c"\x1b[11~");
    e.f_keys[1] = Some(c"\x1b[12~");
    e.f_keys[2] = Some(c"\x1b[13~");
    e.f_keys[3] = Some(c"\x1b[14~");
    e.f_keys[4] = Some(c"\x1b[15~");
    e.f_keys[5] = Some(c"\x1b[17~");
    e.f_keys[6] = Some(c"\x1b[18~");
    e.f_keys[7] = Some(c"\x1b[19~");
    e.f_keys[8] = Some(c"\x1b[20~");
    e.f_keys[9] = Some(c"\x1b[21~");
    e.f_keys[10] = Some(c"\x1b[23~");
    e.f_keys[11] = Some(c"\x1b[24~");
    e.f_keys[12] = Some(c"\x1b[25~");
    e.f_keys[13] = Some(c"\x1b[26~");
    e.f_keys[14] = Some(c"\x1b[28~");
    e.f_keys[15] = Some(c"\x1b[29~");
    e.f_keys[16] = Some(c"\x1b[31~");
    e.f_keys[17] = Some(c"\x1b[32~");
    e.f_keys[18] = Some(c"\x1b[33~");
    e.f_keys[19] = Some(c"\x1b[34~");
    e
};

static RXVT_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[2J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\n");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BLINK_MODE] = Some(c"\x1b[5m");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b7\x1b[?47h");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[m\x0f");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[2J\x1b[?47l\x1b8");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_KEYPAD_LOCAL] = Some(c"\x1b>");
    e.defs[K_TERM_KEYPAD_XMIT] = Some(c"\x1b=");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[K_TERM_SET_A_FOREGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;m%?%p9%t^N%e\x0f%;");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x08"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), Some(c"\x1b[3$")];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[8~"), Some(c"\x1b[8$")];
    e.keys[K_TERM_KEY_FIND] = [Some(c"\x1b[1~"), None];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[7~"), Some(c"\x1b[7$")];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), Some(c"\x1b[2$")];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_SELECT] = [Some(c"\x1b[4~"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1b[D"), Some(c"\x1b[d")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1b[C"), Some(c"\x1b[c")];
    e.f_keys[0] = Some(c"\x1b[11~");
    e.f_keys[1] = Some(c"\x1b[12~");
    e.f_keys[2] = Some(c"\x1b[13~");
    e.f_keys[3] = Some(c"\x1b[14~");
    e.f_keys[4] = Some(c"\x1b[15~");
    e.f_keys[5] = Some(c"\x1b[17~");
    e.f_keys[6] = Some(c"\x1b[18~");
    e.f_keys[7] = Some(c"\x1b[19~");
    e.f_keys[8] = Some(c"\x1b[20~");
    e.f_keys[9] = Some(c"\x1b[21~");
    e.f_keys[10] = Some(c"\x1b[23~");
    e.f_keys[11] = Some(c"\x1b[24~");
    e.f_keys[12] = Some(c"\x1b[25~");
    e.f_keys[13] = Some(c"\x1b[26~");
    e.f_keys[14] = Some(c"\x1b[28~");
    e.f_keys[15] = Some(c"\x1b[29~");
    e.f_keys[16] = Some(c"\x1b[31~");
    e.f_keys[17] = Some(c"\x1b[32~");
    e.f_keys[18] = Some(c"\x1b[33~");
    e.f_keys[19] = Some(c"\x1b[34~");
    e.f_keys[20] = Some(c"\x1b[23$");
    e.f_keys[21] = Some(c"\x1b[24$");
    e.f_keys[22] = Some(c"\x1b[11^");
    e.f_keys[23] = Some(c"\x1b[12^");
    e.f_keys[24] = Some(c"\x1b[13^");
    e.f_keys[25] = Some(c"\x1b[14^");
    e.f_keys[26] = Some(c"\x1b[15^");
    e.f_keys[27] = Some(c"\x1b[17^");
    e.f_keys[28] = Some(c"\x1b[18^");
    e.f_keys[29] = Some(c"\x1b[19^");
    e.f_keys[30] = Some(c"\x1b[20^");
    e.f_keys[31] = Some(c"\x1b[21^");
    e.f_keys[32] = Some(c"\x1b[23^");
    e.f_keys[33] = Some(c"\x1b[24^");
    e.f_keys[34] = Some(c"\x1b[25^");
    e.f_keys[35] = Some(c"\x1b[26^");
    e.f_keys[36] = Some(c"\x1b[28^");
    e.f_keys[37] = Some(c"\x1b[29^");
    e.f_keys[38] = Some(c"\x1b[31^");
    e.f_keys[39] = Some(c"\x1b[32^");
    e.f_keys[40] = Some(c"\x1b[33^");
    e.f_keys[41] = Some(c"\x1b[34^");
    e.f_keys[42] = Some(c"\x1b[23@");
    e.f_keys[43] = Some(c"\x1b[24@");
    e
};

static SCREEN_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(false, 0x100, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\n");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[34h\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1bM");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BLINK_MODE] = Some(c"\x1b[5m");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[?1049h");
    e.defs[K_TERM_ENTER_DIM_MODE] = Some(c"\x1b[2m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[3m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[m\x0f");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[?1049l");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_KEYPAD_LOCAL] = Some(c"\x1b[?1l\x1b>");
    e.defs[K_TERM_KEYPAD_XMIT] = Some(c"\x1b[?1h\x1b=");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[K_TERM_SET_A_FOREGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"\x1b[0%?%p6%t;1%;%?%p1%t;3%;%?%p2%t;4%;%?%p3%t;7%;%?%p4%t;5%;%?%p5%t;2%;m%?%p9%t^N%e\x0f%;");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x7f"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), None];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[4~"), None];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[1~"), None];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), None];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1bOD"), None];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1bOC"), None];
    e.f_keys[0] = Some(c"\x1bOP");
    e.f_keys[1] = Some(c"\x1bOQ");
    e.f_keys[2] = Some(c"\x1bOR");
    e.f_keys[3] = Some(c"\x1bOS");
    e.f_keys[4] = Some(c"\x1b[15~");
    e.f_keys[5] = Some(c"\x1b[17~");
    e.f_keys[6] = Some(c"\x1b[18~");
    e.f_keys[7] = Some(c"\x1b[19~");
    e.f_keys[8] = Some(c"\x1b[20~");
    e.f_keys[9] = Some(c"\x1b[21~");
    e.f_keys[10] = Some(c"\x1b[23~");
    e.f_keys[11] = Some(c"\x1b[24~");
    e
};

const fn set_std_63_fkeys(f_keys: &mut [Option<&'static CStr>; K_TERMINFO_FUNC_KEY_MAX]) {
    f_keys[0] = Some(c"\x1bOP");
    f_keys[1] = Some(c"\x1bOQ");
    f_keys[2] = Some(c"\x1bOR");
    f_keys[3] = Some(c"\x1bOS");
    f_keys[4] = Some(c"\x1b[15~");
    f_keys[5] = Some(c"\x1b[17~");
    f_keys[6] = Some(c"\x1b[18~");
    f_keys[7] = Some(c"\x1b[19~");
    f_keys[8] = Some(c"\x1b[20~");
    f_keys[9] = Some(c"\x1b[21~");
    f_keys[10] = Some(c"\x1b[23~");
    f_keys[11] = Some(c"\x1b[24~");
    f_keys[12] = Some(c"\x1b[1;2P");
    f_keys[13] = Some(c"\x1b[1;2Q");
    f_keys[14] = Some(c"\x1b[1;2R");
    f_keys[15] = Some(c"\x1b[1;2S");
    f_keys[16] = Some(c"\x1b[15;2~");
    f_keys[17] = Some(c"\x1b[17;2~");
    f_keys[18] = Some(c"\x1b[18;2~");
    f_keys[19] = Some(c"\x1b[19;2~");
    f_keys[20] = Some(c"\x1b[20;2~");
    f_keys[21] = Some(c"\x1b[21;2~");
    f_keys[22] = Some(c"\x1b[23;2~");
    f_keys[23] = Some(c"\x1b[24;2~");
    f_keys[24] = Some(c"\x1b[1;5P");
    f_keys[25] = Some(c"\x1b[1;5Q");
    f_keys[26] = Some(c"\x1b[1;5R");
    f_keys[27] = Some(c"\x1b[1;5S");
    f_keys[28] = Some(c"\x1b[15;5~");
    f_keys[29] = Some(c"\x1b[17;5~");
    f_keys[30] = Some(c"\x1b[18;5~");
    f_keys[31] = Some(c"\x1b[19;5~");
    f_keys[32] = Some(c"\x1b[20;5~");
    f_keys[33] = Some(c"\x1b[21;5~");
    f_keys[34] = Some(c"\x1b[23;5~");
    f_keys[35] = Some(c"\x1b[24;5~");
    f_keys[36] = Some(c"\x1b[1;6P");
    f_keys[37] = Some(c"\x1b[1;6Q");
    f_keys[38] = Some(c"\x1b[1;6R");
    f_keys[39] = Some(c"\x1b[1;6S");
    f_keys[40] = Some(c"\x1b[15;6~");
    f_keys[41] = Some(c"\x1b[17;6~");
    f_keys[42] = Some(c"\x1b[18;6~");
    f_keys[43] = Some(c"\x1b[19;6~");
    f_keys[44] = Some(c"\x1b[20;6~");
    f_keys[45] = Some(c"\x1b[21;6~");
    f_keys[46] = Some(c"\x1b[23;6~");
    f_keys[47] = Some(c"\x1b[24;6~");
    f_keys[48] = Some(c"\x1b[1;3P");
    f_keys[49] = Some(c"\x1b[1;3Q");
    f_keys[50] = Some(c"\x1b[1;3R");
    f_keys[51] = Some(c"\x1b[1;3S");
    f_keys[52] = Some(c"\x1b[15;3~");
    f_keys[53] = Some(c"\x1b[17;3~");
    f_keys[54] = Some(c"\x1b[18;3~");
    f_keys[55] = Some(c"\x1b[19;3~");
    f_keys[56] = Some(c"\x1b[20;3~");
    f_keys[57] = Some(c"\x1b[21;3~");
    f_keys[58] = Some(c"\x1b[23;3~");
    f_keys[59] = Some(c"\x1b[24;3~");
    f_keys[60] = Some(c"\x1b[1;4P");
    f_keys[61] = Some(c"\x1b[1;4Q");
    f_keys[62] = Some(c"\x1b[1;4R");
}

static ST_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[2J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\n");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BLINK_MODE] = Some(c"\x1b[5m");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[?1049h");
    e.defs[K_TERM_ENTER_DIM_MODE] = Some(c"\x1b[2m");
    e.defs[K_TERM_ENTER_ITALICS_MODE] = Some(c"\x1b[3m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_SECURE_MODE] = Some(c"\x1b[8m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_ERASE_CHARS] = Some(c"\x1b[%p1%dX");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[0m");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[?1049l");
    e.defs[K_TERM_FROM_STATUS_LINE] = Some(c"\x07");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_KEYPAD_LOCAL] = Some(c"\x1b[?1l\x1b>");
    e.defs[K_TERM_KEYPAD_XMIT] = Some(c"\x1b[?1h\x1b=");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[K_TERM_SET_A_FOREGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"%?%p9%t\x1b(0%e\x1b(B%;\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;%?%p5%t;2%;%?%p7%t;8%;m");
    e.defs[K_TERM_TO_STATUS_LINE] = Some(c"\x1b]0;");
    e.defs[K_TERM_RESET_CURSOR_STYLE] = Some(c"\x1b[2 q");
    e.defs[K_TERM_SET_CURSOR_STYLE] = Some(c"\x1b[%p1%d q");
    e.defs[K_TERM_ENTER_STRIKETHROUGH_MODE] = Some(c"\x1b[9m");
    e.defs[K_TERM_SET_CURSOR_COLOR] = Some(c"\x1b]12;%p1%s\x07");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x7f"), None];
    e.keys[K_TERM_KEY_CLEAR] = [Some(c"\x1b[3;5~"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[4~"), Some(c"\x1b[1;2F")];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[1~"), Some(c"\x1b[1;2H")];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
    set_std_63_fkeys(&mut e.f_keys);
    e
};

static TMUX_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(false, 0x100, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\n");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[34h\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1bM");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BLINK_MODE] = Some(c"\x1b[5m");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[?1049h");
    e.defs[K_TERM_ENTER_DIM_MODE] = Some(c"\x1b[2m");
    e.defs[K_TERM_ENTER_ITALICS_MODE] = Some(c"\x1b[3m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_SECURE_MODE] = Some(c"\x1b[8m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[m\x0f");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[?1049l");
    e.defs[K_TERM_FROM_STATUS_LINE] = Some(c"\x07");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_KEYPAD_LOCAL] = Some(c"\x1b[?1l\x1b>");
    e.defs[K_TERM_KEYPAD_XMIT] = Some(c"\x1b[?1h\x1b=");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[K_TERM_SET_A_FOREGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;%?%p5%t;2%;%?%p7%t;8%;m%?%p9%t^N%e\x0f%;");
    e.defs[K_TERM_TO_STATUS_LINE] = Some(c"\x1b]0;");
    e.defs[K_TERM_RESET_CURSOR_STYLE] = Some(c"\x1b[2 q");
    e.defs[K_TERM_SET_CURSOR_STYLE] = Some(c"\x1b[%p1%d q");
    e.defs[K_TERM_ENTER_STRIKETHROUGH_MODE] = Some(c"\x1b[9m");
    e.defs[K_TERM_SET_CURSOR_COLOR] = Some(c"\x1b]12;%p1%s\x07");
    e.defs[K_TERM_RESET_CURSOR_COLOR] = Some(c"\x1b]112\x07");
    e.defs[K_TERM_SET_UNDERLINE_STYLE] = Some(c"\x1b[4:%p1%dm");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x7f"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[4~"), Some(c"\x1b[1;2F")];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[1~"), Some(c"\x1b[1;2H")];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
    set_std_63_fkeys(&mut e.f_keys);
    e
};

static VTE_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[2J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\n");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BLINK_MODE] = Some(c"\x1b[5m");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[?1049h\x1b[22;0;0t");
    e.defs[K_TERM_ENTER_DIM_MODE] = Some(c"\x1b[2m");
    e.defs[K_TERM_ENTER_ITALICS_MODE] = Some(c"\x1b[3m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_SECURE_MODE] = Some(c"\x1b[8m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_ERASE_CHARS] = Some(c"\x1b[%p1%dX");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[0m\x0f");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[?1049l\x1b[23;0;0t");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_KEYPAD_LOCAL] = Some(c"\x1b[?1l\x1b>");
    e.defs[K_TERM_KEYPAD_XMIT] = Some(c"\x1b[?1h\x1b=");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[K_TERM_SET_A_FOREGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p4%t;5%;%?%p5%t;2%;%?%p7%t;8%;%?%p1%p3%|%t;7%;m%?%p9%t^N%e\x0f%;");
    e.defs[K_TERM_RESET_CURSOR_STYLE] = Some(c"\x1b[1 q");
    e.defs[K_TERM_SET_CURSOR_STYLE] = Some(c"\x1b[%p1%d q");
    e.defs[K_TERM_ENTER_STRIKETHROUGH_MODE] = Some(c"\x1b[9m");
    e.defs[K_TERM_SET_CURSOR_COLOR] = Some(c"\x1b]12;%p1%s\x07");
    e.defs[K_TERM_RESET_CURSOR_COLOR] = Some(c"\x1b]112\x07");
    e.defs[K_TERM_SET_UNDERLINE_STYLE] = Some(c"\x1b[4:%p1%dm");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x7f"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1bOF"), Some(c"\x1b[1;2F")];
    e.keys[K_TERM_KEY_FIND] = [Some(c"\x1b[1~"), None];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1bOH"), Some(c"\x1b[1;2H")];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_SELECT] = [Some(c"\x1b[4~"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
    set_std_63_fkeys(&mut e.f_keys);
    e
};

static XTERM_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[2J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\n");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[?12l\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BLINK_MODE] = Some(c"\x1b[5m");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[?1049h\x1b[22;0;0t");
    e.defs[K_TERM_ENTER_DIM_MODE] = Some(c"\x1b[2m");
    e.defs[K_TERM_ENTER_ITALICS_MODE] = Some(c"\x1b[3m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_SECURE_MODE] = Some(c"\x1b[8m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_ERASE_CHARS] = Some(c"\x1b[%p1%dX");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b(B\x1b[m");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[?1049l\x1b[23;0;0t");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_KEYPAD_LOCAL] = Some(c"\x1b[?1l\x1b>");
    e.defs[K_TERM_KEYPAD_XMIT] = Some(c"\x1b[?1h\x1b=");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[K_TERM_SET_A_FOREGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"%?%p9%t\x1b(0%e\x1b(B%;\x1b[0%?%p6%t;1%;%?%p5%t;2%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;%?%p7%t;8%;m");
    e.defs[K_TERM_SET_LR_MARGIN] = Some(c"\x1b[?69h\x1b[%i%p1%d;%p2%ds");
    e.defs[K_TERM_RESET_CURSOR_STYLE] = Some(c"\x1b[2 q");
    e.defs[K_TERM_SET_CURSOR_STYLE] = Some(c"\x1b[%p1%d q");
    e.defs[K_TERM_ENTER_STRIKETHROUGH_MODE] = Some(c"\x1b[9m");
    e.defs[K_TERM_SET_CURSOR_COLOR] = Some(c"\x1b]12;%p1%s\x07");
    e.defs[K_TERM_RESET_CURSOR_COLOR] = Some(c"\x1b]112\x07");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x08"), None];
    e.keys[K_TERM_KEY_BEG] = [Some(c"\x1bOE"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1bOF"), Some(c"\x1b[1;2F")];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1bOH"), Some(c"\x1b[1;2H")];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
    set_std_63_fkeys(&mut e.f_keys);
    e
};

static CYGWIN_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(false, 8, -1, -1);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\x1b[B");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b7\x1b[?47h");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_SECURE_MODE] = Some(c"\x1b[8m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[0;10m");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[2J\x1b[?47l\x1b8");
    e.defs[K_TERM_FROM_STATUS_LINE] = Some(c"\x07");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] = Some(c"\x1b[4%p1%dm");
    e.defs[K_TERM_SET_A_FOREGROUND] = Some(c"\x1b[3%p1%dm");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"\x1b[0;10%?%p1%t;7%;%?%p2%t;4%;%?%p3%t;7%;%?%p6%t;1%;%?%p7%t;8%;%?%p9%t;11%;m");
    e.defs[K_TERM_TO_STATUS_LINE] = Some(c"\x1b];");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x08"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), None];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[4~"), None];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[1~"), None];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), None];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_SUSPEND] = [Some(c"^Z"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1b[D"), None];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1b[C"), None];
    e.f_keys[0] = Some(c"\x1b[[A");
    e.f_keys[1] = Some(c"\x1b[[B");
    e.f_keys[2] = Some(c"\x1b[[C");
    e.f_keys[3] = Some(c"\x1b[[D");
    e.f_keys[4] = Some(c"\x1b[[E");
    e.f_keys[5] = Some(c"\x1b[17~");
    e.f_keys[6] = Some(c"\x1b[18~");
    e.f_keys[7] = Some(c"\x1b[19~");
    e.f_keys[8] = Some(c"\x1b[20~");
    e.f_keys[9] = Some(c"\x1b[21~");
    e.f_keys[10] = Some(c"\x1b[23~");
    e.f_keys[11] = Some(c"\x1b[24~");
    e.f_keys[12] = Some(c"\x1b[25~");
    e.f_keys[13] = Some(c"\x1b[26~");
    e.f_keys[14] = Some(c"\x1b[28~");
    e.f_keys[15] = Some(c"\x1b[29~");
    e.f_keys[16] = Some(c"\x1b[31~");
    e.f_keys[17] = Some(c"\x1b[32~");
    e.f_keys[18] = Some(c"\x1b[33~");
    e.f_keys[19] = Some(c"\x1b[34~");
    e
};

static WIN32CON_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(false, 8, -1, -1);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\x1b[B");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b7\x1b[?47h");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[0m");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[2J\x1b[?47l\x1b8");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] = Some(c"\x1b[4%p1%dm");
    e.defs[K_TERM_SET_A_FOREGROUND] = Some(c"\x1b[3%p1%dm");
    e.defs[K_TERM_SET_ATTRIBUTES] = Some(c"\x1b[0%?%p1%t;7%;%?%p3%t;7%;%?%p6%t;1%;m");
    e.defs[K_TERM_RESET_CURSOR_STYLE] = Some(c"\x1b[0 q");
    e.defs[K_TERM_SET_CURSOR_STYLE] = Some(c"\x1b[%p1%d q");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x08"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[4~"), Some(c"\x1b[4;2~")];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[1~"), Some(c"\x1b[1;2~")];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_SUSPEND] = [Some(c"^Z"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1b[D"), Some(c"\x1b[1;2D")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1b[C"), Some(c"\x1b[1;2C")];
    e.f_keys[0] = Some(c"\x1b[[A");
    e.f_keys[1] = Some(c"\x1b[[B");
    e.f_keys[2] = Some(c"\x1b[[C");
    e.f_keys[3] = Some(c"\x1b[[D");
    e.f_keys[4] = Some(c"\x1b[[E");
    e.f_keys[5] = Some(c"\x1b[17~");
    e.f_keys[6] = Some(c"\x1b[18~");
    e.f_keys[7] = Some(c"\x1b[19~");
    e.f_keys[8] = Some(c"\x1b[20~");
    e.f_keys[9] = Some(c"\x1b[21~");
    e.f_keys[10] = Some(c"\x1b[23~");
    e.f_keys[11] = Some(c"\x1b[24~");
    e.f_keys[12] = Some(c"\x1b[25~");
    e.f_keys[13] = Some(c"\x1b[26~");
    e.f_keys[14] = Some(c"\x1b[28~");
    e.f_keys[15] = Some(c"\x1b[29~");
    e.f_keys[16] = Some(c"\x1b[31~");
    e.f_keys[17] = Some(c"\x1b[32~");
    e.f_keys[18] = Some(c"\x1b[33~");
    e.f_keys[19] = Some(c"\x1b[34~");
    e.f_keys[20] = Some(c"\x1b[23$");
    e.f_keys[21] = Some(c"\x1b[24$");
    e.f_keys[22] = Some(c"\x1b[11^");
    e.f_keys[23] = Some(c"\x1b[12^");
    e.f_keys[24] = Some(c"\x1b[13^");
    e.f_keys[25] = Some(c"\x1b[14^");
    e.f_keys[26] = Some(c"\x1b[15^");
    e.f_keys[27] = Some(c"\x1b[17^");
    e.f_keys[28] = Some(c"\x1b[18^");
    e.f_keys[29] = Some(c"\x1b[19^");
    e.f_keys[30] = Some(c"\x1b[20^");
    e.f_keys[31] = Some(c"\x1b[21^");
    e.f_keys[32] = Some(c"\x1b[23^");
    e.f_keys[33] = Some(c"\x1b[24^");
    e.f_keys[34] = Some(c"\x1b[25^");
    e.f_keys[35] = Some(c"\x1b[26^");
    e.f_keys[36] = Some(c"\x1b[28^");
    e.f_keys[37] = Some(c"\x1b[29^");
    e.f_keys[38] = Some(c"\x1b[31^");
    e.f_keys[40] = Some(c"\x1b[32^");
    e.f_keys[41] = Some(c"\x1b[33^");
    e.f_keys[42] = Some(c"\x1b[34^");
    e.f_keys[43] = Some(c"\x1b[23@");
    e.f_keys[44] = Some(c"\x1b[24@");
    e
};

static CONEMU_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[2J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\x1b[B");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[?1049h");
    e.defs[K_TERM_ENTER_ITALICS_MODE] = Some(c"\x1b[3m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_ERASE_CHARS] = Some(c"\x1b[%p1%dX");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[0m");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[?1049l");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] = Some(c"\x1b[48;5;%p1%dm");
    e.defs[K_TERM_SET_A_FOREGROUND] = Some(c"\x1b[38;5;%p1%dm");
    e.defs[K_TERM_SET_ATTRIBUTES] = Some(c"\x1b[0%?%p1%p3%|%t;7%;%?%p2%t;4%;%?%p6%t;1%;m");
    e.defs[K_TERM_RESET_CURSOR_STYLE] = Some(c"\x1b[2 q");
    e.defs[K_TERM_SET_CURSOR_STYLE] = Some(c"\x1b[%p1%d q");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x08"), None];
    e.keys[K_TERM_KEY_BEG] = [Some(c"\x1bOE"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[4~"), Some(c"\x1b[4;2~")];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[1~"), Some(c"\x1b[1;2~")];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1b[D"), Some(c"\x1b[1;2D")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1b[C"), Some(c"\x1b[1;2C")];
    e.f_keys[0] = Some(c"\x1b[[A");
    e.f_keys[1] = Some(c"\x1b[[B");
    e.f_keys[2] = Some(c"\x1b[[C");
    e.f_keys[3] = Some(c"\x1b[[D");
    e.f_keys[4] = Some(c"\x1b[[E");
    e.f_keys[5] = Some(c"\x1b[17~");
    e.f_keys[6] = Some(c"\x1b[18~");
    e.f_keys[7] = Some(c"\x1b[19~");
    e.f_keys[8] = Some(c"\x1b[20~");
    e.f_keys[9] = Some(c"\x1b[21~");
    e.f_keys[10] = Some(c"\x1b[23~");
    e.f_keys[11] = Some(c"\x1b[24~");
    e.f_keys[12] = Some(c"\x1b[25~");
    e.f_keys[13] = Some(c"\x1b[26~");
    e.f_keys[14] = Some(c"\x1b[28~");
    e.f_keys[15] = Some(c"\x1b[29~");
    e.f_keys[16] = Some(c"\x1b[31~");
    e.f_keys[17] = Some(c"\x1b[32~");
    e.f_keys[18] = Some(c"\x1b[33~");
    e.f_keys[19] = Some(c"\x1b[34~");
    e.f_keys[20] = Some(c"\x1b[23$");
    e.f_keys[21] = Some(c"\x1b[24$");
    e.f_keys[22] = Some(c"\x1b[11^");
    e.f_keys[23] = Some(c"\x1b[12^");
    e.f_keys[24] = Some(c"\x1b[13^");
    e.f_keys[25] = Some(c"\x1b[14^");
    e.f_keys[26] = Some(c"\x1b[15^");
    e.f_keys[27] = Some(c"\x1b[17^");
    e.f_keys[28] = Some(c"\x1b[18^");
    e.f_keys[29] = Some(c"\x1b[19^");
    e.f_keys[30] = Some(c"\x1b[20^");
    e.f_keys[31] = Some(c"\x1b[21^");
    e.f_keys[32] = Some(c"\x1b[23^");
    e.f_keys[33] = Some(c"\x1b[24^");
    e.f_keys[34] = Some(c"\x1b[25^");
    e.f_keys[35] = Some(c"\x1b[26^");
    e.f_keys[36] = Some(c"\x1b[28^");
    e.f_keys[37] = Some(c"\x1b[29^");
    e.f_keys[38] = Some(c"\x1b[31^");
    e.f_keys[39] = Some(c"\x1b[1;6S");
    e.f_keys[40] = Some(c"\x1b[32^");
    e.f_keys[41] = Some(c"\x1b[33^");
    e.f_keys[42] = Some(c"\x1b[34^");
    e.f_keys[43] = Some(c"\x1b[23@");
    e.f_keys[44] = Some(c"\x1b[24@");
    e
};

static VTPCON_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[K_TERM_CARRIAGE_RETURN] = Some(c"\r");
    e.defs[K_TERM_CHANGE_SCROLL_REGION] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[K_TERM_CLEAR_SCREEN] = Some(c"\x1b[H\x1b[2J");
    e.defs[K_TERM_CLR_EOL] = Some(c"\x1b[K");
    e.defs[K_TERM_CLR_EOS] = Some(c"\x1b[J");
    e.defs[K_TERM_CURSOR_ADDRESS] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[K_TERM_CURSOR_DOWN] = Some(c"\x1b[B");
    e.defs[K_TERM_CURSOR_INVISIBLE] = Some(c"\x1b[?25l");
    e.defs[K_TERM_CURSOR_LEFT] = Some(c"\x08");
    e.defs[K_TERM_CURSOR_HOME] = Some(c"\x1b[H");
    e.defs[K_TERM_CURSOR_NORMAL] = Some(c"\x1b[?12l\x1b[?25h");
    e.defs[K_TERM_CURSOR_UP] = Some(c"\x1b[A");
    e.defs[K_TERM_CURSOR_RIGHT] = Some(c"\x1b[C");
    e.defs[K_TERM_DELETE_LINE] = Some(c"\x1b[M");
    e.defs[K_TERM_ENTER_BOLD_MODE] = Some(c"\x1b[1m");
    e.defs[K_TERM_ENTER_CA_MODE] = Some(c"\x1b[?1049h");
    e.defs[K_TERM_ENTER_ITALICS_MODE] = Some(c"\x1b[3m");
    e.defs[K_TERM_ENTER_REVERSE_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_STANDOUT_MODE] = Some(c"\x1b[7m");
    e.defs[K_TERM_ENTER_UNDERLINE_MODE] = Some(c"\x1b[4m");
    e.defs[K_TERM_ERASE_CHARS] = Some(c"\x1b[%p1%dX");
    e.defs[K_TERM_EXIT_ATTRIBUTE_MODE] = Some(c"\x1b[0m");
    e.defs[K_TERM_EXIT_CA_MODE] = Some(c"\x1b[?1049l");
    e.defs[K_TERM_FROM_STATUS_LINE] = Some(c"\x07");
    e.defs[K_TERM_INSERT_LINE] = Some(c"\x1b[L");
    e.defs[K_TERM_PARM_DELETE_LINE] = Some(c"\x1b[%p1%dM");
    e.defs[K_TERM_PARM_DOWN_CURSOR] = Some(c"\x1b[%p1%dB");
    e.defs[K_TERM_PARM_INSERT_LINE] = Some(c"\x1b[%p1%dL");
    e.defs[K_TERM_PARM_LEFT_CURSOR] = Some(c"\x1b[%p1%dD");
    e.defs[K_TERM_PARM_RIGHT_CURSOR] = Some(c"\x1b[%p1%dC");
    e.defs[K_TERM_PARM_UP_CURSOR] = Some(c"\x1b[%p1%dA");
    e.defs[K_TERM_SET_A_BACKGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[K_TERM_SET_A_FOREGROUND] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[K_TERM_SET_ATTRIBUTES] =
        Some(c"\x1b[0%?%p1%p3%|%t;7%;%?%p2%t;4%;%?%p6%t;1%;%?%p9%t;9%;m");
    e.defs[K_TERM_TO_STATUS_LINE] = Some(c"\x1b]0;");
    e.defs[K_TERM_RESET_CURSOR_STYLE] = Some(c"\x1b[2 q");
    e.defs[K_TERM_SET_CURSOR_STYLE] = Some(c"\x1b[%p1%d q");
    e.defs[K_TERM_ENTER_STRIKETHROUGH_MODE] = Some(c"\x1b[9m");
    e.keys[K_TERM_KEY_BACKSPACE] = [Some(c"\x08"), None];
    e.keys[K_TERM_KEY_BEG] = [Some(c"\x1bOE"), None];
    e.keys[K_TERM_KEY_BTAB] = [Some(c"\x1b[Z"), None];
    e.keys[K_TERM_KEY_DC] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[K_TERM_KEY_END] = [Some(c"\x1b[4~"), Some(c"\x1b[4;2~")];
    e.keys[K_TERM_KEY_HOME] = [Some(c"\x1b[1~"), Some(c"\x1b[1;2~")];
    e.keys[K_TERM_KEY_IC] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[K_TERM_KEY_NPAGE] = [Some(c"\x1b[6~"), None];
    e.keys[K_TERM_KEY_PPAGE] = [Some(c"\x1b[5~"), None];
    e.keys[K_TERM_KEY_LEFT] = [Some(c"\x1b[D"), Some(c"\x1b[1;2D")];
    e.keys[K_TERM_KEY_RIGHT] = [Some(c"\x1b[C"), Some(c"\x1b[1;2C")];
    e.f_keys[0] = Some(c"\x1b[[A");
    e.f_keys[1] = Some(c"\x1b[[B");
    e.f_keys[2] = Some(c"\x1b[[C");
    e.f_keys[3] = Some(c"\x1b[[D");
    e.f_keys[4] = Some(c"\x1b[[E");
    e.f_keys[5] = Some(c"\x1b[17~");
    e.f_keys[6] = Some(c"\x1b[18~");
    e.f_keys[7] = Some(c"\x1b[19~");
    e.f_keys[8] = Some(c"\x1b[20~");
    e.f_keys[9] = Some(c"\x1b[21~");
    e.f_keys[10] = Some(c"\x1b[23~");
    e.f_keys[11] = Some(c"\x1b[24~");
    e.f_keys[12] = Some(c"\x1b[25~");
    e.f_keys[13] = Some(c"\x1b[26~");
    e.f_keys[14] = Some(c"\x1b[28~");
    e.f_keys[15] = Some(c"\x1b[29~");
    e.f_keys[16] = Some(c"\x1b[31~");
    e.f_keys[17] = Some(c"\x1b[32~");
    e.f_keys[18] = Some(c"\x1b[33~");
    e.f_keys[19] = Some(c"\x1b[34~");
    e.f_keys[20] = Some(c"\x1b[23$");
    e.f_keys[21] = Some(c"\x1b[24$");
    e.f_keys[22] = Some(c"\x1b[11^");
    e.f_keys[23] = Some(c"\x1b[12^");
    e.f_keys[24] = Some(c"\x1b[13^");
    e.f_keys[25] = Some(c"\x1b[14^");
    e.f_keys[26] = Some(c"\x1b[15^");
    e.f_keys[27] = Some(c"\x1b[17^");
    e.f_keys[28] = Some(c"\x1b[18^");
    e.f_keys[29] = Some(c"\x1b[19^");
    e.f_keys[30] = Some(c"\x1b[20^");
    e.f_keys[31] = Some(c"\x1b[21^");
    e.f_keys[32] = Some(c"\x1b[23^");
    e.f_keys[33] = Some(c"\x1b[24^");
    e.f_keys[34] = Some(c"\x1b[25^");
    e.f_keys[35] = Some(c"\x1b[26^");
    e.f_keys[36] = Some(c"\x1b[28^");
    e.f_keys[37] = Some(c"\x1b[29^");
    e.f_keys[38] = Some(c"\x1b[31^");
    e.f_keys[39] = Some(c"\x1b[1;6S");
    e.f_keys[40] = Some(c"\x1b[32^");
    e.f_keys[41] = Some(c"\x1b[33^");
    e.f_keys[42] = Some(c"\x1b[34^");
    e.f_keys[43] = Some(c"\x1b[23@");
    e.f_keys[44] = Some(c"\x1b[24@");
    e
};

static UNI_IDS: [UnibiString; 41] = [
    UnibiString::UnibiCarriageReturn,
    UnibiString::UnibiChangeScrollRegion,
    UnibiString::UnibiClearScreen,
    UnibiString::UnibiClrEol,
    UnibiString::UnibiClrEos,
    UnibiString::UnibiCursorAddress,
    UnibiString::UnibiCursorDown,
    UnibiString::UnibiCursorInvisible,
    UnibiString::UnibiCursorLeft,
    UnibiString::UnibiCursorHome,
    UnibiString::UnibiCursorNormal,
    UnibiString::UnibiCursorUp,
    UnibiString::UnibiCursorRight,
    UnibiString::UnibiDeleteLine,
    UnibiString::UnibiEnterBlinkMode,
    UnibiString::UnibiEnterBoldMode,
    UnibiString::UnibiEnterCaMode,
    UnibiString::UnibiEnterDimMode,
    UnibiString::UnibiEnterItalicsMode,
    UnibiString::UnibiEnterReverseMode,
    UnibiString::UnibiEnterSecureMode,
    UnibiString::UnibiEnterStandoutMode,
    UnibiString::UnibiEnterUnderlineMode,
    UnibiString::UnibiEraseChars,
    UnibiString::UnibiExitAttributeMode,
    UnibiString::UnibiExitCaMode,
    UnibiString::UnibiFromStatusLine,
    UnibiString::UnibiInsertLine,
    UnibiString::UnibiKeypadLocal,
    UnibiString::UnibiKeypadXmit,
    UnibiString::UnibiParmDeleteLine,
    UnibiString::UnibiParmDownCursor,
    UnibiString::UnibiParmInsertLine,
    UnibiString::UnibiParmLeftCursor,
    UnibiString::UnibiParmRightCursor,
    UnibiString::UnibiParmUpCursor,
    UnibiString::UnibiSetABackground,
    UnibiString::UnibiSetAForeground,
    UnibiString::UnibiSetAttributes,
    UnibiString::UnibiSetLrMargin,
    UnibiString::UnibiToStatusLine,
];

static UNI_EXT: [&CStr; 8] = [
    c"Se", c"Ss", c"smxx", c"setrgbf", c"setrgbb", c"Cs", c"Cr", c"Smulx",
];

static UNI_KEYS: [[UnibiString; 2]; K_TERM_KEY_COUNT] = [
    [
        UnibiString::UnibiKeyBackspace,
        UnibiString::UnibiStringBeginMarker,
    ],
    [UnibiString::UnibiKeyBeg, UnibiString::UnibiKeySbeg],
    [
        UnibiString::UnibiKeyBtab,
        UnibiString::UnibiStringBeginMarker,
    ],
    [
        UnibiString::UnibiKeyClear,
        UnibiString::UnibiStringBeginMarker,
    ],
    [UnibiString::UnibiKeyDc, UnibiString::UnibiKeySdc],
    [UnibiString::UnibiKeyEnd, UnibiString::UnibiKeySend],
    [UnibiString::UnibiKeyFind, UnibiString::UnibiKeySfind],
    [UnibiString::UnibiKeyHome, UnibiString::UnibiKeyShome],
    [UnibiString::UnibiKeyIc, UnibiString::UnibiKeySic],
    [
        UnibiString::UnibiKeyNpage,
        UnibiString::UnibiStringBeginMarker,
    ],
    [
        UnibiString::UnibiKeyPpage,
        UnibiString::UnibiStringBeginMarker,
    ],
    [
        UnibiString::UnibiKeySelect,
        UnibiString::UnibiStringBeginMarker,
    ],
    [
        UnibiString::UnibiKeySuspend,
        UnibiString::UnibiKeySsuspend,
    ],
    [UnibiString::UnibiKeyUndo, UnibiString::UnibiKeySundo],
    [UnibiString::UnibiKeyLeft, UnibiString::UnibiKeySleft],
    [
        UnibiString::UnibiKeyRight,
        UnibiString::UnibiKeySright,
    ],
];

static UNI_FKEYS: [UnibiString; K_TERMINFO_FUNC_KEY_MAX] = [
    UnibiString::UnibiKeyF1,
    UnibiString::UnibiKeyF2,
    UnibiString::UnibiKeyF3,
    UnibiString::UnibiKeyF4,
    UnibiString::UnibiKeyF5,
    UnibiString::UnibiKeyF6,
    UnibiString::UnibiKeyF7,
    UnibiString::UnibiKeyF8,
    UnibiString::UnibiKeyF9,
    UnibiString::UnibiKeyF10,
    UnibiString::UnibiKeyF11,
    UnibiString::UnibiKeyF12,
    UnibiString::UnibiKeyF13,
    UnibiString::UnibiKeyF14,
    UnibiString::UnibiKeyF15,
    UnibiString::UnibiKeyF16,
    UnibiString::UnibiKeyF17,
    UnibiString::UnibiKeyF18,
    UnibiString::UnibiKeyF19,
    UnibiString::UnibiKeyF20,
    UnibiString::UnibiKeyF21,
    UnibiString::UnibiKeyF22,
    UnibiString::UnibiKeyF23,
    UnibiString::UnibiKeyF24,
    UnibiString::UnibiKeyF25,
    UnibiString::UnibiKeyF26,
    UnibiString::UnibiKeyF27,
    UnibiString::UnibiKeyF28,
    UnibiString::UnibiKeyF29,
    UnibiString::UnibiKeyF30,
    UnibiString::UnibiKeyF31,
    UnibiString::UnibiKeyF32,
    UnibiString::UnibiKeyF33,
    UnibiString::UnibiKeyF34,
    UnibiString::UnibiKeyF35,
    UnibiString::UnibiKeyF36,
    UnibiString::UnibiKeyF37,
    UnibiString::UnibiKeyF38,
    UnibiString::UnibiKeyF39,
    UnibiString::UnibiKeyF40,
    UnibiString::UnibiKeyF41,
    UnibiString::UnibiKeyF42,
    UnibiString::UnibiKeyF43,
    UnibiString::UnibiKeyF44,
    UnibiString::UnibiKeyF45,
    UnibiString::UnibiKeyF46,
    UnibiString::UnibiKeyF47,
    UnibiString::UnibiKeyF48,
    UnibiString::UnibiKeyF49,
    UnibiString::UnibiKeyF50,
    UnibiString::UnibiKeyF51,
    UnibiString::UnibiKeyF52,
    UnibiString::UnibiKeyF53,
    UnibiString::UnibiKeyF54,
    UnibiString::UnibiKeyF55,
    UnibiString::UnibiKeyF56,
    UnibiString::UnibiKeyF57,
    UnibiString::UnibiKeyF58,
    UnibiString::UnibiKeyF59,
    UnibiString::UnibiKeyF60,
    UnibiString::UnibiKeyF61,
    UnibiString::UnibiKeyF62,
    UnibiString::UnibiKeyF63,
];

static STRING_NAMES: [&str; K_TERM_COUNT] = [
    "carriage_return",
    "change_scroll_region",
    "clear_screen",
    "clr_eol",
    "clr_eos",
    "cursor_address",
    "cursor_down",
    "cursor_invisible",
    "cursor_left",
    "cursor_home",
    "cursor_normal",
    "cursor_up",
    "cursor_right",
    "delete_line",
    "enter_blink_mode",
    "enter_bold_mode",
    "enter_ca_mode",
    "enter_dim_mode",
    "enter_italics_mode",
    "enter_reverse_mode",
    "enter_secure_mode",
    "enter_standout_mode",
    "enter_underline_mode",
    "erase_chars",
    "exit_attribute_mode",
    "exit_ca_mode",
    "from_status_line",
    "insert_line",
    "keypad_local",
    "keypad_xmit",
    "parm_delete_line",
    "parm_down_cursor",
    "parm_insert_line",
    "parm_left_cursor",
    "parm_right_cursor",
    "parm_up_cursor",
    "set_a_background",
    "set_a_foreground",
    "set_attributes",
    "set_lr_margin",
    "to_status_line",
    "reset_cursor_style (Se)",
    "set_cursor_style (Ss)",
    "enter_strikethrough_mode (smxx)",
    "set_rgb_foreground (setrgbf)",
    "set_rgb_background (setrgbb)",
    "set_cursor_color (Cs)",
    "reset_cursor_color (Cr)",
    "set_underline_style (Smulx)",
];

static KEY_NAMES: [&str; K_TERM_KEY_COUNT] = [
    "backspace",
    "beg",
    "btab",
    "clear",
    "dc",
    "end",
    "find",
    "home",
    "ic",
    "npage",
    "ppage",
    "select",
    "suspend",
    "undo",
    "left",
    "right",
];

static FKEY_NAMES: [&str; K_TERMINFO_FUNC_KEY_MAX] = [
    "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "f13", "f14", "f15",
    "f16", "f17", "f18", "f19", "f20", "f21", "f22", "f23", "f24", "f25", "f26", "f27", "f28",
    "f29", "f30", "f31", "f32", "f33", "f34", "f35", "f36", "f37", "f38", "f39", "f40", "f41",
    "f42", "f43", "f44", "f45", "f46", "f47", "f48", "f49", "f50", "f51", "f52", "f53", "f54",
    "f55", "f56", "f57", "f58", "f59", "f60", "f61", "f62", "f63",
];

const MAGIC_16BIT: u16 = 0o0432;
const MAGIC_32BIT: u16 = 0o1036;
const MAX15BITS: u16 = 0x7fff;
const MAX31BITS: u32 = 0x7fff_ffff;
const UNIBI_MAX_BUF: usize = 32768;
const UNIBI_BOOL_COUNT: usize = (UnibiBoolean::UnibiBooleanEndMarker as usize)
    - (UnibiBoolean::UnibiBooleanBeginMarker as usize)
    - 1;
const UNIBI_NUM_COUNT: usize = (UnibiNumeric::UnibiNumericEndMarker as usize)
    - (UnibiNumeric::UnibiNumericBeginMarker as usize)
    - 1;
const UNIBI_STR_COUNT: usize =
    (UnibiString::UnibiStringEndMarker as usize) - (UnibiString::UnibiStringBeginMarker as usize) - 1;

const UNIBI_TERMINFO: &str = if cfg!(windows) {
    ""
} else {
    "/usr/share/terminfo"
};

const UNIBI_TERMINFO_DIRS: &str = if cfg!(windows) {
    ""
} else {
    "/etc/terminfo:/lib/terminfo:/usr/share/terminfo:/usr/lib/terminfo:/usr/local/share/terminfo:/usr/local/lib/terminfo"
};

struct UnibiTerm {
    bools: [bool; UNIBI_BOOL_COUNT],
    nums: [c_int; UNIBI_NUM_COUNT],
    str_offs: [i16; UNIBI_STR_COUNT],
    str_table: Vec<u8>,
    ext_bool_count: usize,
    ext_num_count: usize,
    ext_str_offs: Vec<i16>,
    ext_name_offs: Vec<usize>,
    ext_table: Vec<u8>,
}

#[inline]
fn get_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

#[inline]
fn get_short16(buf: &[u8], off: usize) -> i16 {
    let n = get_u16_le(buf, off);
    if n <= MAX15BITS { n as i16 } else { -1 }
}

#[inline]
fn get_int32(buf: &[u8], off: usize) -> c_int {
    let n = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
    if n <= MAX31BITS { n as c_int } else { -1 }
}

impl UnibiTerm {
    fn from_mem(data: &[u8]) -> Option<Self> {
        if data.len() < 12 {
            return None;
        }
        let magic = get_u16_le(data, 0);
        if magic != MAGIC_16BIT && magic != MAGIC_32BIT {
            return None;
        }
        let numsize: usize = if magic == MAGIC_16BIT { 2 } else { 4 };
        let namlen = get_u16_le(data, 2) as usize;
        let boollen = get_u16_le(data, 4) as usize;
        let numlen = get_u16_le(data, 6) as usize;
        let strslen = get_u16_le(data, 8) as usize;
        let tablsz = get_u16_le(data, 10) as usize;

        let mut pos = 12usize;
        if data.len() - pos < namlen {
            return None;
        }
        pos += namlen;

        if data.len() - pos < boollen {
            return None;
        }
        let mut bools = [false; UNIBI_BOOL_COUNT];
        for (i, slot) in bools.iter_mut().enumerate().take(boollen) {
            *slot = data[pos + i] != 0;
        }
        pos += boollen;
        if !(namlen + boollen).is_multiple_of(2) && pos < data.len() {
            pos += 1;
        }

        if data.len() - pos < numlen * numsize {
            return None;
        }
        let mut nums = [-1; UNIBI_NUM_COUNT];
        for (i, slot) in nums.iter_mut().enumerate().take(numlen) {
            *slot = if numsize == 2 {
                c_int::from(get_short16(data, pos + i * 2))
            } else {
                get_int32(data, pos + i * 4)
            };
        }
        pos += numlen * numsize;

        if data.len() - pos < strslen * 2 {
            return None;
        }
        let mut str_offs = [-1i16; UNIBI_STR_COUNT];
        for (i, slot) in str_offs.iter_mut().enumerate().take(strslen) {
            *slot = get_short16(data, pos + i * 2);
        }
        pos += strslen * 2;

        if data.len() - pos < tablsz {
            return None;
        }
        let mut str_table = data[pos..pos + tablsz].to_vec();
        if let Some(last) = str_table.last_mut() {
            *last = 0;
        }
        pos += tablsz;
        if !tablsz.is_multiple_of(2) && pos < data.len() {
            pos += 1;
        }

        let mut ext_bool_count = 0usize;
        let mut ext_num_count = 0usize;
        let mut ext_str_offs = Vec::new();
        let mut ext_name_offs = Vec::new();
        let mut ext_table = Vec::new();

        if data.len() - pos >= 10 {
            let extboollen = get_u16_le(data, pos);
            let extnumlen = get_u16_le(data, pos + 2);
            let extstrslen = get_u16_le(data, pos + 4);
            let extofflen = get_u16_le(data, pos + 6);
            let exttablsz = get_u16_le(data, pos + 8);

            if extboollen <= MAX15BITS
                && extnumlen <= MAX15BITS
                && extstrslen <= MAX15BITS
                && extofflen <= MAX15BITS
                && exttablsz <= MAX15BITS
            {
                pos += 10;
                let eb = extboollen as usize;
                let en = extnumlen as usize;
                let es = extstrslen as usize;
                let et = exttablsz as usize;
                let extalllen = eb + en + es;
                let needed = eb + (eb % 2) + en * numsize + es * 2 + extalllen * 2 + et;
                if data.len() - pos < needed {
                    return None;
                }

                ext_bool_count = eb;
                pos += eb + (eb % 2);

                ext_num_count = en;
                pos += en * numsize;

                let tbl1_pos = pos + (es + extalllen) * 2;
                let tbl1 = &data[tbl1_pos..tbl1_pos + et];
                let mut s_max = 0usize;
                let mut s_sum = 0usize;

                ext_str_offs.reserve(es);
                for i in 0..es {
                    let v = get_short16(data, pos + i * 2);
                    if v < 0 || (v as usize) >= et {
                        ext_str_offs.push(-1);
                    } else {
                        let start = v as usize;
                        let rem = &tbl1[start..];
                        let end_off = match rem.iter().position(|&b| b == 0) {
                            Some(nul_idx) => start + nul_idx + 1,
                            None => et,
                        };
                        s_sum += end_off - start;
                        s_max = s_max.max(end_off);
                        ext_str_offs.push(v);
                    }
                }
                pos += es * 2;

                if s_max != s_sum {
                    return None;
                }

                let tblsz2 = et - s_sum;
                ext_name_offs.reserve(extalllen);
                for i in 0..extalllen {
                    let v = get_short16(data, pos + i * 2);
                    if v < 0 || (v as usize) >= tblsz2 {
                        return None;
                    }
                    ext_name_offs.push(s_sum + (v as usize));
                }

                ext_table = tbl1.to_vec();
                if let Some(last) = ext_table.last_mut() {
                    *last = 0;
                }
            }
        }

        Some(Self {
            bools,
            nums,
            str_offs,
            str_table,
            ext_bool_count,
            ext_num_count,
            ext_str_offs,
            ext_name_offs,
            ext_table,
        })
    }

    #[inline]
    fn get_bool(&self, v: UnibiBoolean) -> bool {
        let idx = (v as usize).wrapping_sub((UnibiBoolean::UnibiBooleanBeginMarker as usize) + 1);
        self.bools.get(idx).copied().unwrap_or(false)
    }

    #[inline]
    fn get_num(&self, v: UnibiNumeric) -> c_int {
        let idx = (v as usize).wrapping_sub((UnibiNumeric::UnibiNumericBeginMarker as usize) + 1);
        self.nums.get(idx).copied().unwrap_or(-1)
    }

    #[inline]
    fn get_str(&self, v: UnibiString) -> *const c_char {
        let idx = (v as usize).wrapping_sub((UnibiString::UnibiStringBeginMarker as usize) + 1);
        let off = self.str_offs.get(idx).copied().unwrap_or(-1);
        if off < 0 || (off as usize) >= self.str_table.len() {
            ptr::null()
        } else {
            unsafe { self.str_table.as_ptr().add(off as usize).cast::<c_char>() }
        }
    }

    #[inline]
    fn count_ext_bool(&self) -> usize {
        self.ext_bool_count
    }

    #[inline]
    fn get_ext_bool_name(&self, i: usize) -> *const c_char {
        if i >= self.ext_bool_count {
            return ptr::null();
        }
        let off = self.ext_name_offs[i];
        unsafe { self.ext_table.as_ptr().add(off).cast::<c_char>() }
    }

    #[inline]
    fn count_ext_str(&self) -> usize {
        self.ext_str_offs.len()
    }

    #[inline]
    fn get_ext_str_name(&self, i: usize) -> *const c_char {
        if i >= self.ext_str_offs.len() {
            return ptr::null();
        }
        let off = self.ext_name_offs[self.ext_bool_count + self.ext_num_count + i];
        unsafe { self.ext_table.as_ptr().add(off).cast::<c_char>() }
    }

    #[inline]
    fn get_ext_str(&self, i: usize) -> *const c_char {
        let off = self.ext_str_offs.get(i).copied().unwrap_or(-1);
        if off < 0 || (off as usize) >= self.ext_table.len() {
            ptr::null()
        } else {
            unsafe { self.ext_table.as_ptr().add(off as usize).cast::<c_char>() }
        }
    }
}

fn unibi_from_file(path: &str) -> Result<Option<UnibiTerm>, std::io::ErrorKind> {
    let mut f = std::fs::File::open(path).map_err(|e| e.kind())?;
    let mut buf = Vec::with_capacity(4096);
    f.by_ref()
        .take(UNIBI_MAX_BUF as u64)
        .read_to_end(&mut buf)
        .map_err(|e| e.kind())?;
    Ok(UnibiTerm::from_mem(&buf))
}

fn unibi_from_dir(
    dir: &str,
    mid: Option<&str>,
    term: &str,
) -> Result<Option<UnibiTerm>, std::io::ErrorKind> {
    let first_byte = term.as_bytes()[0];
    let first_ch = first_byte as char;
    let prefix = match mid {
        Some(m) => format!("{dir}/{m}"),
        None => dir.to_string(),
    };
    let path1 = format!("{prefix}/{first_ch}/{term}");
    match unibi_from_file(&path1) {
        Ok(Some(ut)) => Ok(Some(ut)),
        Ok(None) => Err(std::io::ErrorKind::InvalidData),
        Err(std::io::ErrorKind::NotFound) => {
            let path2 = format!("{prefix}/{first_byte:02x}/{term}");
            match unibi_from_file(&path2) {
                Ok(Some(ut)) => Ok(Some(ut)),
                Ok(None) => Err(std::io::ErrorKind::InvalidData),
                Err(k) => Err(k),
            }
        }
        Err(k) => Err(k),
    }
}

fn is_ignorable_dir_err(k: std::io::ErrorKind) -> bool {
    matches!(
        k,
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
    )
}

fn unibi_from_dirs(list: &str, term: &str) -> Option<UnibiTerm> {
    if list.is_empty() {
        return None;
    }
    let mut unibi_terminfo_searched = UNIBI_TERMINFO.is_empty();
    for entry in list.split(':') {
        let res = if !entry.is_empty() {
            unibi_from_dir(entry, None, term)
        } else if !unibi_terminfo_searched {
            unibi_terminfo_searched = true;
            unibi_from_dir(UNIBI_TERMINFO, None, term)
        } else {
            continue;
        };
        match res {
            Ok(Some(ut)) => return Some(ut),
            Ok(None) => return None,
            Err(k) if !is_ignorable_dir_err(k) => return None,
            Err(_) => {}
        }
    }
    None
}

fn get_env_str(name: &CStr) -> Option<String> {
    crate::os::env::os_getenv_cstr(name.as_ptr()).map(|c| c.to_string_lossy().into_owned())
}

fn unibi_from_term(termname: impl crate::strings::IntoCCharPtr) -> Option<UnibiTerm> { let termname = termname.into_c_char_ptr();
    if termname.is_null() {
        return None;
    }
    let term_bytes =  { crate::memory::cstr_as_bytes(termname) };
    if term_bytes.is_empty() || term_bytes[0] == b'.' || term_bytes.contains(&b'/') {
        return None;
    }
    let term = std::str::from_utf8(term_bytes).ok()?;

    if let Some(env) = get_env_str(c"TERMINFO")
        && let Ok(Some(ut)) = unibi_from_dir(&env, None, term)
    {
        return Some(ut);
    }

    if let Some(home) = get_env_str(c"HOME") {
        match unibi_from_dir(&home, Some(".terminfo"), term) {
            Ok(Some(ut)) => return Some(ut),
            Ok(None) => return None,
            Err(k) if !is_ignorable_dir_err(k) => return None,
            Err(_) => {}
        }
    }

    if let Some(dirs) = get_env_str(c"TERMINFO_DIRS")
        && let Some(ut) = unibi_from_dirs(&dirs, term)
    {
        return Some(ut);
    }

    unibi_from_dirs(UNIBI_TERMINFO_DIRS, term)
}

pub fn terminfo_is_term_family(term: impl crate::strings::IntoCCharPtr, family: &std::ffi::CStr) -> bool { let term = term.into_c_char_ptr();
    if term.is_null() {
        return false;
    }
     {
        let term_bytes = crate::memory::cstr_as_bytes(term);
        let fam_bytes = family.to_bytes();
        let tlen = term_bytes.len();
        let flen = fam_bytes.len();
        tlen >= flen
            && &term_bytes[..flen] == fam_bytes
            && (tlen == flen || term_bytes[flen] == b'-' || term_bytes[flen] == b'.')
    }
}

pub fn terminfo_is_bsd_console(term: impl crate::strings::IntoCCharPtr) -> bool { let term = term.into_c_char_ptr();
    #[cfg(any(
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    unsafe {
        if (crate::memory::cstr_as_opt_bytes(term) == Some(b"vt220")) || (crate::memory::cstr_as_opt_bytes(term) == Some(b"vt100")) {
            return true;
        }
        #[cfg(target_os = "freebsd")]
        {
            return (crate::memory::cstr_as_opt_bytes(term) == Some(b"xterm"))
                && os_env_exists(c"XTERM_VERSION", true);
        }
    }
    let _ = term;
    false
}

pub fn terminfo_from_builtin(
    term: impl crate::strings::IntoCCharPtr,
    termname: &mut Option<std::ffi::CString>,
) -> TerminfoEntry { let term = term.into_c_char_ptr();
    if (crate::memory::cstr_as_opt_bytes(term) == Some(b"ghostty")) || (crate::memory::cstr_as_opt_bytes(term) == Some(b"xterm-ghostty")) {
            *termname = Some(c"ghostty".to_owned());
            GHOSTTY_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"xterm") {
            *termname = Some(c"xterm".to_owned());
            XTERM_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"screen") {
            *termname = Some(c"screen".to_owned());
            SCREEN_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"tmux") {
            *termname = Some(c"tmux".to_owned());
            TMUX_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"rxvt") {
            *termname = Some(c"rxvt".to_owned());
            RXVT_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"putty") {
            *termname = Some(c"putty".to_owned());
            PUTTY_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"linux") {
            *termname = Some(c"linux".to_owned());
            LINUX_16COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"interix") {
            *termname = Some(c"interix".to_owned());
            INTERIX_8COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"iterm")
            || terminfo_is_term_family(term, c"iterm2")
            || terminfo_is_term_family(term, c"iTerm.app")
            || terminfo_is_term_family(term, c"iTerm2.app")
        {
            *termname = Some(c"iterm".to_owned());
            ITERM_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"st") {
            *termname = Some(c"st".to_owned());
            ST_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"gnome")
            || terminfo_is_term_family(term, c"vte")
        {
            *termname = Some(c"vte".to_owned());
            VTE_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"cygwin") {
            *termname = Some(c"cygwin".to_owned());
            CYGWIN_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"win32con") {
            *termname = Some(c"win32con".to_owned());
            WIN32CON_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"conemu") {
            *termname = Some(c"conemu".to_owned());
            CONEMU_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"vtpcon") {
            *termname = Some(c"vtpcon".to_owned());
            VTPCON_TERMINFO.to_entry()
        } else {
            *termname = Some(c"ansi".to_owned());
            ANSI_TERMINFO.to_entry()
        }
}

pub fn terminfo_from_database(ti: &mut TerminfoEntry, termname: impl crate::strings::IntoCCharPtr) -> bool { let termname = termname.into_c_char_ptr();
     {
        let Some(ut) = unibi_from_term(termname) else {
            return false;
        };

        ti.back_color_erase = ut.get_bool(UnibiBoolean::UnibiBackColorErase);
        ti.max_colors = ut.get_num(UnibiNumeric::UnibiMaxColors);
        ti.lines = ut.get_num(UnibiNumeric::UnibiLines);
        ti.columns = ut.get_num(UnibiNumeric::UnibiColumns);

        ti.Tc = false;
        ti.RGB = false;
        ti.Su = false;
        let ext_bool_count = ut.count_ext_bool();
        for i in 0..ext_bool_count {
            let n = ut.get_ext_bool_name(i);
            if !n.is_null() {
                let bytes = crate::memory::cstr_as_bytes(n);
                if bytes == b"Tc" {
                    ti.Tc = true;
                } else if bytes == b"RGB" {
                    ti.RGB = true;
                } else if bytes == b"Su" {
                    ti.Su = true;
                }
            }
        }

        for (i, &uni_id) in UNI_IDS.iter().enumerate() {
            let val = ut.get_str(uni_id);
            ti.defs[i] = if !val.is_null() {
                Some(Cow::Owned(crate::memory::cstr_to_cstring(val)))
            } else {
                None
            };
        }

        let max = ut.count_ext_str();
        for (i, &name) in UNI_EXT.iter().enumerate() {
            for val in 0..max {
                let n = ut.get_ext_str_name(val);
                if !n.is_null() && (crate::memory::cstr_as_opt_bytes(n) == crate::memory::cstr_as_opt_bytes(name.as_ptr())) {
                    let data = ut.get_ext_str(val);
                    ti.defs[K_TERM_EXT_OFFSET + i] = if !data.is_null() {
                        Some(Cow::Owned(crate::memory::cstr_to_cstring(data)))
                    } else {
                        None
                    };
                    break;
                }
            }
        }

        for (i, pair) in UNI_KEYS.iter().enumerate() {
            let val = ut.get_str(pair[0]);
            if !val.is_null() {
                ti.keys[i][0] = Some(Cow::Owned(crate::memory::cstr_to_cstring(val)));
                if pair[1] != UnibiString::UnibiStringBeginMarker {
                    let sval = ut.get_str(pair[1]);
                    ti.keys[i][1] = if !sval.is_null() {
                        Some(Cow::Owned(crate::memory::cstr_to_cstring(sval)))
                    } else {
                        None
                    };
                }
            }
        }

        for (i, &fkey_id) in UNI_FKEYS.iter().enumerate() {
            let val = ut.get_str(fkey_id);
            ti.f_keys[i] = if !val.is_null() {
                Some(Cow::Owned(crate::memory::cstr_to_cstring(val)))
            } else {
                None
            };
        }

        true
    }
}

#[inline]
const fn fmt_bool(val: bool) -> &'static str {
    if val { "true" } else { "false" }
}

pub fn terminfo_info_msg(
    ti: &TerminfoEntry,
    termname: Option<&CStr>,
    from_db: bool,
) -> Vec<u8> {
    let mut data: Vec<u8> = Vec::new();
        let termname_bytes = termname.map_or(&b""[..], CStr::to_bytes);

        data.extend_from_slice(b"&term: ");
        data.extend_from_slice(termname_bytes);
        data.push(b'\n');
        if from_db {
            data.extend_from_slice(b"using terminfo database\n\n");
        } else {
            data.extend_from_slice(b"using builtin terminfo\n\n");
        }

        let header = format!(
            "Boolean capabilities:\n  back_color_erase: {}\n  truecolor ('Tc'): {}\n  RGB: {}\n  extended underline ('Su'): {}\n\nNumeric capabilities: (-1 for unknown)\n  lines: {}\n  columns: {}\n  max_colors: {}\n\nString capabilities:\n",
            fmt_bool(ti.back_color_erase),
            fmt_bool(ti.Tc),
            fmt_bool(ti.RGB),
            fmt_bool(ti.Su),
            ti.lines,
            ti.columns,
            ti.max_colors
        );
        data.extend_from_slice(header.as_bytes());

        for (i, &name) in STRING_NAMES.iter().enumerate() {
            if let Some(ref s) = ti.defs[i] {
                let prefix = format!("  {name:<31} = ");
                data.extend_from_slice(prefix.as_bytes());
                kv_transstr(&mut data, s.as_ptr(), false);
                data.push(b'\n');
            }
        }

        for (i, &name) in KEY_NAMES.iter().enumerate() {
            if let Some(ref s) = ti.keys[i][0] {
                let prefix = format!("  key_{name:<27} = ");
                data.extend_from_slice(prefix.as_bytes());
                kv_transstr(&mut data, s.as_ptr(), false);
                if let Some(ref ss) = ti.keys[i][1] {
                    let sprefix = format!(", key_s{name} = ");
                    data.extend_from_slice(sprefix.as_bytes());
                    kv_transstr(&mut data, ss.as_ptr(), false);
                }
                data.push(b'\n');
            }
        }

        for (i, &name) in FKEY_NAMES.iter().enumerate() {
            if let Some(ref s) = ti.f_keys[i] {
                let prefix = format!("  key_{name:<27} = ");
                data.extend_from_slice(prefix.as_bytes());
                kv_transstr(&mut data, s.as_ptr(), false);
                data.push(b'\n');
            }
        }

        data
}

fn push(num: c_long, string: Option<[c_char; 8]>, stack: &mut TPSTACK) -> c_int {
    if stack.offset >= stack.nums.len() {
        return -1;
    }
    stack.nums[stack.offset] = num;
    stack.strings[stack.offset] = string;
    stack.offset += 1;
    0
}

fn pop(num: *mut c_long, string: Option<&mut Option<[c_char; 8]>>, stack: &mut TPSTACK) -> c_int {
    unsafe {
        if stack.offset == 0 {
            if !num.is_null() {
                *num = 0;
            }
            if let Some(s) = string {
                *s = None;
            }
            return -1;
        }
        stack.offset -= 1;
        if !num.is_null() {
            *num = stack.nums[stack.offset];
        }
        if let Some(s) = string {
            *s = stack.strings[stack.offset];
        }
        0
    }
}

fn ochar(buf: *mut *mut c_char, buf_end: impl crate::strings::IntoCCharPtr, mut c: c_int) -> bool { let buf_end = buf_end.into_c_char_ptr();
    unsafe {
        if c == 0 {
            c = 0o200;
        }
        if buf_end.offset_from(*buf) < 2 {
            return false;
        }
        **buf = c as c_char;
        *buf = (*buf).add(1);
        true
    }
}

fn onum(
    buf: *mut *mut c_char,
    buf_end: impl crate::strings::IntoCCharPtr,
    fmt: impl crate::strings::IntoCCharPtr,
    num: c_int,
    mut len: usize,
) -> bool { let buf_end = buf_end.into_c_char_ptr(); let fmt = fmt.into_c_char_ptr();
    const LONG_STR_MAX: usize = 21;
    len = len.max(LONG_STR_MAX);

    unsafe {
        if buf_end.offset_from(*buf) < (len + 2) as isize {
            return false;
        }
        {
            let l = vim_snprintf!(*buf, len + 2, fmt, num);
            if l < 0 {
                return false;
            }
            *buf = (*buf).add(l as usize);
        }
        true
    }
}

pub fn terminfo_fmt(
    buf_start: *mut c_char,
    buf_end: *mut c_char,
    str_: impl crate::strings::IntoCCharPtr,
    params: *mut TPVAR,
) -> usize { let mut str_ = str_.into_c_char_ptr();
    unsafe {
        let mut fmt = [0 as c_char; 64];
        let mut ostr: Option<[c_char; 8]> = None;
        let mut val: c_long;
        let mut val2: c_long = 0;
        let mut dnums = [0 as c_long; 26];
        let mut snums = [0 as c_long; 26];

        let mut buf = buf_start;
        let mut stack = TPSTACK {
            nums: [0; 20],
            strings: [None; 20],
            offset: 0,
        };

        loop {
            let mut c = *str_;
            str_ = str_.add(1);
            if c == NUL {
                break;
            }
            if c != b'%' as c_char || {
                c = *str_;
                str_ = str_.add(1);
                c == b'%' as c_char
            } {
                if c == NUL {
                    break;
                }
                if !ochar(&mut buf, buf_end, c as c_int) {
                    return 0;
                }
                continue;
            }

            let mut fp_idx: usize = 0;
            fmt[fp_idx] = b'%' as c_char;
            fp_idx += 1;
            let mut done: u32 = 0;
            let mut dot: u32 = 0;
            let mut minus: u32 = 0;
            let mut width: usize = 0;
            let mut precision: usize = 0;
            val = 0;

            while done == 0 && fp_idx < fmt.len() {
                match c as u8 {
                    b'c' | b's' => {
                        fmt[fp_idx] = c;
                        fp_idx += 1;
                        done = 1;
                    }
                    b'd' | b'o' | b'x' | b'X' => {
                        fmt[fp_idx] = b'l' as c_char;
                        fp_idx += 1;
                        fmt[fp_idx] = c;
                        fp_idx += 1;
                        done = 1;
                    }
                    b'#' | b' ' => {
                        fmt[fp_idx] = c;
                        fp_idx += 1;
                    }
                    b'.' => {
                        fmt[fp_idx] = c;
                        fp_idx += 1;
                        if dot == 0 {
                            dot = 1;
                            width = val as usize;
                        } else {
                            done = 2;
                        }
                        val = 0;
                    }
                    b':' => {
                        minus = 1;
                    }
                    b'-' => {
                        if minus != 0 {
                            fmt[fp_idx] = c;
                            fp_idx += 1;
                        } else {
                            done = 1;
                        }
                    }
                    b if b.is_ascii_digit() => {
                        val = (val * 10) + c_long::from(b - b'0');
                        if val > 10000 {
                            done = 2;
                        } else {
                            fmt[fp_idx] = c;
                            fp_idx += 1;
                        }
                    }
                    _ => {
                        done = 1;
                    }
                }
                if done == 0 {
                    c = *str_;
                    str_ = str_.add(1);
                }
            }

            let olen = if done == 2 {
                fp_idx = 1;
                fmt[fp_idx] = *str_;
                0
            } else {
                if dot == 0 {
                    width = val as usize;
                } else {
                    precision = val as usize;
                }
                width.max(precision)
            };
            fmt[fp_idx] = NUL;

            match c as u8 {
                b'c' => {
                    pop(&mut val, None, &mut stack);
                    if !ochar(&mut buf, buf_end, (val as u8) as c_int) {
                        return 0;
                    }
                }
                b's' => {
                    pop(ptr::null_mut(), Some(&mut ostr), &mut stack);
                    if let Some(ref s_arr) = ostr {
                        let ostr_ptr = s_arr.as_ptr();
                        let mut l = crate::memory::cstr_len(ostr_ptr);
                        if l < olen {
                            l = olen;
                        }
                        if (buf_end.offset_from(buf) as usize) < (l + 1) {
                            return 0;
                        }
                        {
                            let r = vim_snprintf!(buf, l + 1, fmt.as_ptr(), ostr_ptr);
                            if r >= 0 {
                                buf = buf.add(r as usize);
                            }
                        }
                    }
                }
                b'l' => {
                    pop(ptr::null_mut(), Some(&mut ostr), &mut stack);
                    let l = ostr.map_or(0, |s_arr| {
                        crate::memory::cstr_len(s_arr.as_ptr())
                    });
                    push(l as c_long, None, &mut stack);
                }
                b'd' | b'o' | b'x' | b'X' => {
                    pop(&mut val, None, &mut stack);
                    if !onum(&mut buf, buf_end, fmt.as_ptr(), val as c_int, olen) {
                        return 0;
                    }
                }
                b'p' => {
                    let sc = crate::memory::cstr_first_byte(str_);
                    if !(b'1'..=b'9').contains(&sc) {
                        continue;
                    }
                    str_ = str_.add(1);
                    let l = (sc - b'1') as usize;
                    let param = *params.add(l);
                    if push(param.num, param.string, &mut stack) != 0 {
                        return 0;
                    }
                }
                b'P' => {
                    pop(&mut val, None, &mut stack);
                    let sc = crate::memory::cstr_first_byte(str_);
                    if sc.is_ascii_lowercase() {
                        dnums[(sc - b'a') as usize] = val;
                    } else if sc.is_ascii_uppercase() {
                        snums[(sc - b'A') as usize] = val;
                    }
                }
                b'g' => {
                    let sc = crate::memory::cstr_first_byte(str_);
                    if sc.is_ascii_lowercase() {
                        if push(dnums[(sc - b'a') as usize], None, &mut stack) != 0 {
                            return 0;
                        }
                    } else if sc.is_ascii_uppercase()
                        && push(snums[(sc - b'A') as usize], None, &mut stack) != 0
                    {
                        return 0;
                    }
                }
                b'i' => {
                    (*params.add(0)).num += 1;
                    (*params.add(1)).num += 1;
                }
                b'\'' => {
                    let ch = crate::memory::cstr_first_byte(str_);
                    str_ = str_.add(1);
                    if push(c_long::from(ch), None, &mut stack) != 0 {
                        return 0;
                    }
                    while !crate::memory::cstr_is_empty(str_) && crate::memory::cstr_first_byte(str_) != b'\'' {
                        str_ = str_.add(1);
                    }
                    if crate::memory::cstr_first_byte(str_) == b'\'' {
                        str_ = str_.add(1);
                    }
                }
                b'{' => {
                    val = 0;
                    while (crate::memory::cstr_first_byte(str_)).is_ascii_digit() {
                        val = (val * 10) + c_long::from((crate::memory::cstr_first_byte(str_)) - b'0');
                        str_ = str_.add(1);
                    }
                    if push(val, None, &mut stack) != 0 {
                        return 0;
                    }
                    while !crate::memory::cstr_is_empty(str_) && crate::memory::cstr_first_byte(str_) != b'}' {
                        str_ = str_.add(1);
                    }
                    if crate::memory::cstr_first_byte(str_) == b'}' {
                        str_ = str_.add(1);
                    }
                }
                b'+' | b'-' | b'*' | b'/' | b'm' | b'A' | b'O' | b'&' | b'|' | b'^' | b'='
                | b'<' | b'>' => {
                    pop(&mut val, None, &mut stack);
                    pop(&mut val2, None, &mut stack);
                    val = match c as u8 {
                        b'+' => val.wrapping_add(val2),
                        b'-' => val2.wrapping_sub(val),
                        b'*' => val.wrapping_mul(val2),
                        b'/' => {
                            if val != 0 {
                                val2.wrapping_div(val)
                            } else {
                                0
                            }
                        }
                        b'm' => {
                            if val != 0 {
                                val2.wrapping_rem(val)
                            } else {
                                0
                            }
                        }
                        b'A' => c_long::from(val != 0 && val2 != 0),
                        b'O' => c_long::from(val != 0 || val2 != 0),
                        b'&' => val & val2,
                        b'|' => val | val2,
                        b'^' => val ^ val2,
                        b'=' => c_long::from(val == val2),
                        b'<' => c_long::from(val2 < val),
                        b'>' => c_long::from(val2 > val),
                        _ => val,
                    };
                    if push(val, None, &mut stack) != 0 {
                        return 0;
                    }
                }
                b'!' | b'~' => {
                    pop(&mut val, None, &mut stack);
                    val = match c as u8 {
                        b'!' => c_long::from(val == 0),
                        b'~' => !val,
                        _ => val,
                    };
                    if push(val, None, &mut stack) != 0 {
                        return 0;
                    }
                }
                b'?' => {}
                b't' => {
                    pop(&mut val, None, &mut stack);
                    if val == 0 {
                        let mut l: usize = 0;
                        while !crate::memory::cstr_is_empty(str_) {
                            if crate::memory::cstr_first_byte(str_) != b'%' {
                                str_ = str_.add(1);
                                continue;
                            }
                            str_ = str_.add(1);
                            if crate::memory::cstr_first_byte(str_) == b'?' {
                                l += 1;
                            } else if crate::memory::cstr_first_byte(str_) == b';' {
                                if l > 0 {
                                    l -= 1;
                                } else {
                                    str_ = str_.add(1);
                                    break;
                                }
                            } else if crate::memory::cstr_first_byte(str_) == b'e' && l == 0 {
                                str_ = str_.add(1);
                                break;
                            }
                            if !crate::memory::cstr_is_empty(str_) {
                                str_ = str_.add(1);
                            }
                        }
                    }
                }
                b'e' => {
                    let mut l: usize = 0;
                    while !crate::memory::cstr_is_empty(str_) {
                        if crate::memory::cstr_first_byte(str_) != b'%' {
                            str_ = str_.add(1);
                            continue;
                        }
                        str_ = str_.add(1);
                        if crate::memory::cstr_first_byte(str_) == b'?' {
                            l += 1;
                        } else if crate::memory::cstr_first_byte(str_) == b';' {
                            if l > 0 {
                                l -= 1;
                            } else {
                                str_ = str_.add(1);
                                break;
                            }
                        }
                        if !crate::memory::cstr_is_empty(str_) {
                            str_ = str_.add(1);
                        }
                    }
                }
                b';' => {}
                _ => {}
            }
        }
        buf.offset_from(buf_start) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unibi_from_mem_parses_16bit_and_extended_capabilities() {
        let mut data = Vec::new();
        // Header: magic (0432), namlen=6 ("xterm\0"), boollen=1, numlen=1, strslen=3, tablsz=4 ("\r\0")
        data.extend_from_slice(&MAGIC_16BIT.to_le_bytes());
        data.extend_from_slice(&6u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&3u16.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        // Name ("xterm\0") + 1 bool (1) + 1 pad byte
        data.extend_from_slice(b"xterm\0\x01\x00");
        // 1 numeric: columns = 80
        data.extend_from_slice(&80u16.to_le_bytes());
        // 3 strings: back_tab=-1, bell=-1, carriage_return=0
        data.extend_from_slice(&0xffffu16.to_le_bytes());
        data.extend_from_slice(&0xffffu16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        // String table: "\r\0" (2 bytes, even)
        data.extend_from_slice(b"\r\0");

        // Extended header: extboollen=1, extnumlen=0, extstrslen=1, extofflen=3, exttablsz=12
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&3u16.to_le_bytes());
        data.extend_from_slice(&12u16.to_le_bytes());
        // 1 ext bool (1) + 1 pad byte
        data.extend_from_slice(&[1, 0]);
        // 1 ext str offset: 0 ("\x1b[0 q\0", 6 bytes)
        data.extend_from_slice(&0u16.to_le_bytes());
        // 2 ext name offsets (relative to tbl2): "Tc\0" at 0, "Se\0" at 3
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&3u16.to_le_bytes());
        // Ext table: "\x1b[0 q\0" (6 bytes) + "Tc\0Se\0" (6 bytes) = 12 bytes
        data.extend_from_slice(b"\x1b[0 q\0Tc\0Se\0");

        let ut = UnibiTerm::from_mem(&data).expect("valid terminfo");
        assert!(ut.get_bool(UnibiBoolean::UnibiAutoLeftMargin));
        assert!(!ut.get_bool(UnibiBoolean::UnibiBackColorErase));
        assert_eq!(ut.get_num(UnibiNumeric::UnibiColumns), 80);
        assert_eq!(ut.get_num(UnibiNumeric::UnibiMaxColors), -1);
        assert!(ut.get_str(UnibiString::UnibiBackTab).is_null());
         {
            assert_eq!(
                crate::memory::cstr_as_bytes(ut.get_str(UnibiString::UnibiCarriageReturn)),
                b"\r"
            );
            assert_eq!(ut.count_ext_bool(), 1);
            assert_eq!(crate::memory::cstr_as_bytes(ut.get_ext_bool_name(0)), b"Tc");
            assert_eq!(ut.count_ext_str(), 1);
            assert_eq!(crate::memory::cstr_as_bytes(ut.get_ext_str_name(0)), b"Se");
            assert_eq!(crate::memory::cstr_as_bytes(ut.get_ext_str(0)), b"\x1b[0 q");
        }
    }

    #[test]
    fn terminfo_fmt_stack_overflow_fails_before_output() {
        let mut buf = [0 as c_char; 256];
        let mut params = [TPVAR {
            num: 65,
            string: None,
        }; 9];
        let valid_fmt = format!("{}%c\0", "%p1".repeat(20));
        let overflow_fmt = format!("{}%c\0", "%p1".repeat(21));

        unsafe {
            let start = buf.as_mut_ptr();
            let end = start.add(buf.len());
            let valid_n = terminfo_fmt(start, end, valid_fmt.as_ptr().cast(), params.as_mut_ptr());
            assert_eq!(valid_n, 1);
            assert_eq!(buf[0] as u8, b'A');

            let overflow_n = terminfo_fmt(
                start,
                end,
                overflow_fmt.as_ptr().cast(),
                params.as_mut_ptr(),
            );
            assert_eq!(overflow_n, 0);
        }
    }
}
