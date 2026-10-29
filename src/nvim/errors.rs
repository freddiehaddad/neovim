//! Shared error messages (`src/nvim/errors.h`).

macro_rules! def_err {
    ($name:ident, $s:literal) => {
        pub static $name: &std::ffi::CStr = match std::ffi::CStr::from_bytes_with_nul($s) {
            Ok(s) => s,
            Err(_) => panic!("invalid CStr literal"),
        };
    };
}

#[cfg(test)]
def_err!(E_ABORT, b"E470: Command aborted\0");
#[cfg(test)]
def_err!(E_AFTERINIT, b"E905: Cannot set this option after startup\0");
def_err!(E_API_SPAWN_FAILED, b"E903: Could not spawn API job\0");
def_err!(E_ARGREQ, b"E471: Argument required\0");
def_err!(E_BACKSLASH, b"E10: \\ should be followed by /, ? or &\0");
def_err!(
    E_CURDIR,
    b"E12: Command not allowed in secure mode in current dir or tag search\0"
);
def_err!(
    E_INVALID_BUFFER_NAME_STR,
    b"E158: Invalid buffer name: %s\0"
);
def_err!(E_COMMAND_TOO_RECURSIVE, b"E169: Command too recursive\0");
def_err!(
    E_BUFFER_NR_INVALID_BUFFER_NUMBER,
    b"E680: <buffer=%d>: invalid buffer number\0"
);
def_err!(E_BUFFER_IS_NOT_LOADED, b"E681: Buffer is not loaded\0");
def_err!(
    E_BUFFER_CHANGED_UNEXPECTEDLY,
    b"E787: Buffer changed unexpectedly\0"
);
def_err!(E_ENDIF, b"E171: Missing :endif\0");
def_err!(E_ENDTRY, b"E600: Missing :endtry\0");
def_err!(E_ENDWHILE, b"E170: Missing :endwhile\0");
def_err!(E_ENDFOR, b"E170: Missing :endfor\0");
def_err!(E_WHILE, b"E588: :endwhile without :while\0");
def_err!(E_FOR, b"E588: :endfor without :for\0");
def_err!(E_EXISTS, b"E13: File exists (add ! to override)\0");
def_err!(E_FAILED, b"E472: Command failed\0");
def_err!(E_INTERN2, b"E685: Internal error: %s\0");
def_err!(E_INTERR, b"Interrupted\0");
def_err!(E_INVARG, b"E474: Invalid argument\0");
def_err!(E_INVARG2, b"E475: Invalid argument: %s\0");
def_err!(E_INVARGVAL, b"E475: Invalid value for argument %s\0");
def_err!(E_INVARG_NVAL, b"E475: Invalid value for argument %s: %s\0");
def_err!(E_DUPARG2, b"E983: Duplicate argument: %s\0");
def_err!(E_INVEXPR2, b"E15: Invalid expression: \"%s\"\0");
def_err!(E_INVRANGE, b"E16: Invalid range\0");
def_err!(
    E_INTERNAL_ERROR_IN_REGEXP,
    b"E473: Internal error in regexp\0"
);
def_err!(E_INVCMD, b"E476: Invalid command\0");
def_err!(E_ISADIR2, b"E17: \"%s\" is a directory\0");
def_err!(E_NO_SPELL, b"E756: Spell checking is not possible\0");
def_err!(E_INVCHAN, b"E900: Invalid channel id\0");
def_err!(E_INVCHANJOB, b"E900: Invalid channel id: not a job\0");
#[cfg(test)]
def_err!(E_JOBTBLFULL, b"E901: Job table is full\0");
def_err!(E_JOBSPAWN, b"E903: Process failed to start: %s: \"%s\"\0");
def_err!(E_CHANNOTPTY, b"E904: channel is not a pty\0");
def_err!(E_STDIOCHAN2, b"E905: Couldn't open stdio channel: %s\0");
def_err!(E_INVSTREAM, b"E906: invalid stream for channel\0");
def_err!(
    E_INVSTREAMRPC,
    b"E906: invalid stream for rpc channel, use 'rpc'\0"
);
#[cfg(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
))]
def_err!(
    E_STREAMKEY,
    b"E5210: dict key '%s' already set for buffered stream in channel %lu\0"
);
#[cfg(not(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
)))]
def_err!(
    E_STREAMKEY,
    b"E5210: dict key '%s' already set for buffered stream in channel %llu\0"
);
def_err!(E_LIBCALL, b"E364: Library call failed for \"%s()\"\0");
def_err!(E_FSYNC, b"E667: Fsync failed: %s\0");
def_err!(E_MKDIR, b"E739: Cannot create directory %s: %s\0");
def_err!(E_MARKINVAL, b"E19: Mark has invalid line number\0");
def_err!(E_MARKNOTSET, b"E20: Mark not set\0");
def_err!(
    E_MODIFIABLE,
    b"E21: Cannot make changes, 'modifiable' is off\0"
);
def_err!(E_NESTING, b"E22: Scripts nested too deep\0");
def_err!(E_NOALT, b"E23: No alternate file\0");
def_err!(E_NOABBR, b"E24: No such abbreviation\0");
def_err!(E_NOBANG, b"E477: No ! allowed\0");
def_err!(E_NOGROUP, b"E28: No such highlight group name: %s\0");
def_err!(E_NOINSTEXT, b"E29: No inserted text yet\0");
def_err!(E_NOLASTCMD, b"E30: No previous command line\0");
def_err!(E_NOMAP, b"E31: No such mapping\0");
def_err!(E_NOIDENT, b"E349: No identifier under cursor\0");
def_err!(E_NOMATCH, b"E479: No match\0");
def_err!(E_NOMATCH2, b"E480: No match: %s\0");
def_err!(E_NONAME, b"E32: No file name\0");
def_err!(
    E_NOPRESUB,
    b"E33: No previous substitute regular expression\0"
);
def_err!(E_NOPREV, b"E34: No previous command\0");
def_err!(E_NOPREVRE, b"E35: No previous regular expression\0");
def_err!(E_NORANGE, b"E481: No range allowed\0");
def_err!(E_NOROOM, b"E36: Not enough room\0");
def_err!(E_NOTMP, b"E483: Can't get temp file name\0");
def_err!(E_NOTOPEN, b"E484: Can't open file %s\0");
def_err!(E_NOTOPEN_2, b"E484: Can't open file %s: %s\0");
def_err!(E_CANT_READ_FILE_STR, b"E485: Can't read file %s\0");
def_err!(E_NULL, b"E38: Null argument\0");
def_err!(E_NUMBER_EXP, b"E39: Number expected\0");
def_err!(E_OPENERRF, b"E40: Can't open errorfile %s\0");
def_err!(E_OUTOFMEM, b"E41: Out of memory!\0");
def_err!(E_PATNOTF2, b"E486: Pattern not found: %s\0");
def_err!(E_POSITIVE, b"E487: Argument must be positive\0");
def_err!(E_PREV_DIR, b"E459: Cannot go back to previous directory\0");
def_err!(E_NO_ERRORS, b"E42: No Errors\0");
def_err!(E_LOCLIST, b"E776: No location list\0");
def_err!(E_RE_DAMG, b"E43: Damaged match string\0");
def_err!(E_RE_CORR, b"E44: Corrupted regexp program\0");
def_err!(
    E_READONLY,
    b"E45: 'readonly' option is set (add ! to override)\0"
);
def_err!(E_LETWRONG, b"E734: Wrong variable type for %s=\0");
def_err!(E_ILLVAR, b"E461: Illegal variable name: %s\0");
def_err!(E_CANNOT_MOD, b"E995: Cannot modify existing variable\0");
def_err!(
    E_CANNOT_CHANGE_READONLY_VARIABLE_STR,
    b"E46: Cannot change read-only variable \"%.*s\"\0"
);
def_err!(E_DICTREQ, b"E715: Dictionary required\0");
#[cfg(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
))]
def_err!(E_BLOBIDX, b"E979: Blob index out of range: %ld\0");
#[cfg(not(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
)))]
def_err!(E_BLOBIDX, b"E979: Blob index out of range: %lld\0");
def_err!(E_INVALBLOB, b"E978: Invalid operation for Blob\0");
def_err!(E_TOOMANYARG, b"E118: Too many arguments for function: %s\0");
def_err!(
    E_TOOFEWARG,
    b"E119: Not enough arguments for function: %s\0"
);
def_err!(E_DICTKEY, b"E716: Key not present in Dictionary: \"%s\"\0");
def_err!(
    E_DICTKEY_LEN,
    b"E716: Key not present in Dictionary: \"%.*s\"\0"
);
def_err!(E_LISTREQ, b"E714: List required\0");
def_err!(E_LISTBLOBREQ, b"E897: List or Blob required\0");
def_err!(
    E_LISTBLOBARG,
    b"E899: Argument of %s must be a List or Blob\0"
);
def_err!(
    E_LISTDICTARG,
    b"E712: Argument of %s must be a List or Dictionary\0"
);
def_err!(
    E_LISTDICTBLOBARG,
    b"E896: Argument of %s must be a List, Dictionary or Blob\0"
);
def_err!(E_READERRF, b"E47: Error while reading errorfile\0");
def_err!(E_SANDBOX, b"E48: Not allowed in sandbox\0");
def_err!(E_SECURE, b"E523: Not allowed here\0");
def_err!(
    E_TEXTLOCK,
    b"E565: Not allowed to change text or change window\0"
);
def_err!(E_SCREENMODE, b"E359: Screen mode setting not supported\0");
def_err!(E_SCROLL, b"E49: Invalid scroll size\0");
def_err!(E_SHELLEMPTY, b"E91: 'shell' option is empty\0");
#[cfg(test)]
def_err!(E_SIGNDATA, b"E255: Couldn't read in sign data!\0");
def_err!(E_SWAPCLOSE, b"E72: Close error on swap file\0");
def_err!(E_COMMAND_TOO_COMPLEX, b"E74: Command too complex\0");
def_err!(E_LONGNAME, b"E75: Name too long\0");
def_err!(E_TOOMSBRA, b"E76: Too many [\0");
def_err!(E_TOOMANY, b"E77: Too many file names (glob not allowed)\0");
def_err!(E_TRAILING, b"E488: Trailing characters\0");
def_err!(E_TRAILING_ARG, b"E488: Trailing characters: %s\0");
def_err!(E_UMARK, b"E78: Unknown mark\0");
#[cfg(any(unix, test))]
def_err!(E_WILDEXPAND, b"E79: Cannot expand wildcards\0");
def_err!(
    E_WINHEIGHT,
    b"E591: 'winheight' cannot be smaller than 'winminheight'\0"
);
def_err!(
    E_WINWIDTH,
    b"E592: 'winwidth' cannot be smaller than 'winminwidth'\0"
);
def_err!(E_WRITE, b"E80: Error while writing\0");
def_err!(E_ZEROCOUNT, b"E939: Positive count required\0");
def_err!(E_USINGSID, b"E81: Using <SID> not in a script context\0");
def_err!(E_MISSINGPAREN, b"E107: Missing parentheses: %s\0");
def_err!(E_EMPTY_BUFFER, b"E749: Empty buffer\0");
#[cfg(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
))]
def_err!(E_NOBUFNR, b"E86: Buffer %ld does not exist\0");
#[cfg(not(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
)))]
def_err!(E_NOBUFNR, b"E86: Buffer %lld does not exist\0");
def_err!(
    E_NO_WRITE_SINCE_LAST_CHANGE,
    b"E37: No write since last change\0"
);
def_err!(
    E_NO_WRITE_SINCE_LAST_CHANGE_ADD_BANG_TO_OVERRIDE,
    b"E37: No write since last change (add ! to override)\0"
);
def_err!(
    E_NO_WRITE_SINCE_LAST_CHANGE_FOR_BUFFER_NR_ADD_BANG_TO_OVERRIDE,
    b"E89: No write since last change for buffer %d (add ! to override)\0"
);
def_err!(E_BUFFER_NR_NOT_FOUND, b"E92: Buffer %d not found\0");
def_err!(E_UNKNOWN_FUNCTION_STR, b"E117: Unknown function: %s\0");
def_err!(
    E_STR_NOT_INSIDE_FUNCTION,
    b"E193: %s not inside a function\0"
);
def_err!(E_JOB_STILL_RUNNING, b"E948: Job still running\0");
def_err!(
    E_JOB_STILL_RUNNING_ADD_BANG_TO_END_THE_JOB,
    b"E948: Job still running (add ! to end the job)\0"
);
def_err!(E_INVALPAT, b"E682: Invalid search pattern or delimiter\0");
def_err!(E_BUFLOADED, b"E139: File is loaded in another buffer\0");
def_err!(E_NOTSET, b"E764: Option '%s' is not set\0");
#[cfg(test)]
def_err!(E_INVALIDREG, b"E850: Invalid register name\0");
def_err!(E_DIRNOTF, b"E919: Directory not found in '%s': \"%s\"\0");
def_err!(
    E_AU_RECURSIVE,
    b"E952: Autocommand caused recursive behavior\0"
);
def_err!(
    E_MENU_ONLY_EXISTS_IN_ANOTHER_MODE,
    b"E328: Menu only exists in another mode\0"
);
def_err!(E_AUTOCMD_CLOSE, b"E813: Cannot close autocmd window\0");
#[cfg(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
))]
def_err!(
    E_LIST_INDEX_OUT_OF_RANGE_NR,
    b"E684: List index out of range: %ld\0"
);
#[cfg(not(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
)))]
def_err!(
    E_LIST_INDEX_OUT_OF_RANGE_NR,
    b"E684: List index out of range: %lld\0"
);
def_err!(E_LISTARG, b"E686: Argument of %s must be a List\0");
def_err!(E_UNSUPPORTEDOPTION, b"E519: Option not supported\0");
#[cfg(test)]
def_err!(E_FNAMETOOLONG, b"E856: Filename too long\0");
def_err!(
    E_USING_FLOAT_AS_STRING,
    b"E806: Using a Float as a String\0"
);
def_err!(
    E_CANNOT_EDIT_OTHER_BUF,
    b"E788: Not allowed to edit another buffer now\0"
);
def_err!(
    E_USING_NUMBER_AS_BOOL_NR,
    b"E1023: Using a Number as a Bool: %d\0"
);
def_err!(E_NOT_CALLABLE_TYPE_STR, b"E1085: Not a callable type: %s\0");
def_err!(E_AUABORT, b"E855: Autocommands caused command to abort\0");
def_err!(E_API_ERROR, b"E5555: API call: %s\0");
def_err!(
    E_FAST_API_DISABLED,
    b"E5560: %s must not be called in a fast event context\0"
);
def_err!(E_NOUI, b"E5768: No UI attached\0");
def_err!(
    E_FLOATONLY,
    b"E5601: Cannot close window, only floating window would remain\0"
);
def_err!(E_FLOATEXCHANGE, b"E5602: Cannot exchange or rotate float\0");
def_err!(
    E_CANT_FIND_DIRECTORY_STR_IN_CDPATH,
    b"E344: Can't find directory \"%s\" in cdpath\0"
);
def_err!(
    E_CANT_FIND_FILE_STR_IN_PATH,
    b"E345: Can't find file \"%s\" in path\0"
);
def_err!(
    E_NO_MORE_DIRECTORY_STR_FOUND_IN_CDPATH,
    b"E346: No more directory \"%s\" found in cdpath\0"
);
def_err!(
    E_NO_MORE_FILE_STR_FOUND_IN_PATH,
    b"E347: No more file \"%s\" found in path\0"
);
def_err!(E_VALUE_IS_LOCKED, b"E741: Value is locked\0");
def_err!(E_VALUE_IS_LOCKED_STR, b"E741: Value is locked: %.*s\0");
def_err!(E_CANNOT_CHANGE_VALUE, b"E742: Cannot change value\0");
def_err!(
    E_CANNOT_CHANGE_VALUE_OF_STR,
    b"E742: Cannot change value of %.*s\0"
);
def_err!(
    E_CANNOT_SET_VARIABLE_IN_SANDBOX_STR,
    b"E794: Cannot set variable in the sandbox: \"%.*s\"\0"
);
def_err!(
    E_CANNOT_DELETE_VARIABLE_STR,
    b"E795: Cannot delete variable %.*s\0"
);
def_err!(E_INVALWINDOW, b"E957: Invalid window number\0");
def_err!(
    E_PROBLEM_CREATING_INTERNAL_DIFF,
    b"E960: Problem creating the internal diff\0"
);
def_err!(
    E_CANNOT_DEFINE_AUTOCOMMANDS_FOR_ALL_EVENTS,
    b"E1155: Cannot define autocommands for ALL events\0"
);
def_err!(
    E_CANNOT_CHANGE_ARGLIST_RECURSIVELY,
    b"E1156: Cannot change the argument list recursively\0"
);
def_err!(
    E_RESULTING_TEXT_TOO_LONG,
    b"E1240: Resulting text too long\0"
);
def_err!(
    E_LINE_NUMBER_OUT_OF_RANGE,
    b"E1247: Line number out of range\0"
);
def_err!(
    E_HIGHLIGHT_GROUP_NAME_INVALID_CHAR,
    b"E5248: Invalid character in group name\0"
);
def_err!(
    E_HIGHLIGHT_GROUP_NAME_TOO_LONG,
    b"E1249: Highlight group name too long\0"
);
def_err!(E_STRING_REQUIRED, b"E928: String required\0");
def_err!(
    E_INVALID_COLUMN_NUMBER_NR,
    b"E964: Invalid column number: %ld\0"
);
def_err!(
    E_INVALID_LINE_NUMBER_NR,
    b"E966: Invalid line number: %ld\0"
);
def_err!(
    E_REDUCE_OF_AN_EMPTY_STR_WITH_NO_INITIAL_VALUE,
    b"E998: Reduce of an empty %s with no initial value\0"
);
#[cfg(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
))]
def_err!(
    E_INVALID_VALUE_FOR_BLOB_NR,
    b"E1239: Invalid value for blob: 0xlX\0"
);
#[cfg(not(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
)))]
def_err!(
    E_INVALID_VALUE_FOR_BLOB_NR,
    b"E1239: Invalid value for blob: 0xllX\0"
);
def_err!(
    E_STRAY_CLOSING_CURLY_STR,
    b"E1278: Stray '}' without a matching '{': %s\0"
);
def_err!(E_MISSING_CLOSE_CURLY_STR, b"E1279: Missing '}': %s\0");
def_err!(
    E_CANNOT_CHANGE_MENUS_WHILE_LISTING,
    b"E1310: Cannot change menus while listing\0"
);
def_err!(
    E_NOT_ALLOWED_TO_CHANGE_WINDOW_LAYOUT_IN_THIS_AUTOCMD,
    b"E1312: Not allowed to change the window layout in this autocmd\0"
);
def_err!(E_VAL_TOO_LARGE, b"E1510: Value too large: %s\0");
def_err!(E_VAL_TOO_LARGE_LEN, b"E1510: Value too large: %.*s\0");
def_err!(
    E_UNDOBANG_CANNOT_REDO_OR_MOVE_BRANCH,
    b"E5767: Cannot use :undo! to redo or move to a different undo branch\0"
);
def_err!(
    E_WINFIXBUF_CANNOT_GO_TO_BUFFER,
    b"E1513: Cannot switch buffer. 'winfixbuf' is enabled\0"
);
def_err!(
    E_INVALID_RETURN_TYPE_FROM_FINDFUNC,
    b"E1514: 'findfunc' did not return a List type\0"
);
def_err!(
    E_CANNOT_SWITCH_TO_A_CLOSING_BUFFER,
    b"E1546: Cannot switch to a closing buffer\0"
);
def_err!(
    E_CANNOT_HAVE_MORE_THAN_NR_DIFF_ANCHORS,
    b"E1549: Cannot have more than %d diff anchors\0"
);
def_err!(
    E_FAILED_TO_FIND_ALL_DIFF_ANCHORS,
    b"E1550: Failed to find all diff anchors\0"
);
def_err!(
    E_DIFF_ANCHORS_WITH_HIDDEN_WINDOWS,
    b"E1562: Diff anchors cannot be used with hidden diff windows\0"
);
def_err!(
    E_LEADTAB_REQUIRES_TAB,
    b"E1572: 'listchars' field \"leadtab\" requires \"tab\" to be specified\0"
);
def_err!(
    E_INVALID_FORMAT_STRING_SINGLE_PERCENT_S,
    b"E1577: Invalid format string, only one \"%s\" is allowed\0"
);
def_err!(
    E_TOO_MANY_POSTPONED_PREFIXES_SPELL,
    b"E1578: Too many postponed prefixes and/or compound flags\0"
);
def_err!(E_TRUSTFILE, b"E5570: Cannot update trust file: %s\0");
def_err!(E_CANNOT_READ_FROM_STR_2, b"E282: Cannot read from \"%s\"\0");
def_err!(
    E_CONFLICTING_CONFIGS,
    b"E5422: Conflicting configs: \"%s\" \"%s\"\0"
);
def_err!(E_UNKNOWN_OPTION2, b"E355: Unknown option: %s\0");
def_err!(
    E_RESTART_FAILED_CMD_NO_QUIT,
    b"E5201: Restart failed: +cmd did not quit server: %s\0"
);
def_err!(TOP_BOT_MSG, b"search hit TOP, continuing at BOTTOM\0");
def_err!(BOT_TOP_MSG, b"search hit BOTTOM, continuing at TOP\0");
def_err!(LINE_MSG, b" line \0");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_strings_are_nul_terminated_and_match_codes() {
        let abort = E_ABORT;
        assert_eq!(&abort.to_bytes()[4..], b": Command aborted");
        let invarg = E_INVARG;
        assert_eq!(&invarg.to_bytes()[4..], b": Invalid argument");
        let lmsg = LINE_MSG;
        assert_eq!(lmsg.to_bytes(), b" line ");
        for ptr in [
            E_AFTERINIT.as_ptr(),
            E_JOBTBLFULL.as_ptr(),
            E_SIGNDATA.as_ptr(),
            E_WILDEXPAND.as_ptr(),
            E_INVALIDREG.as_ptr(),
            E_FNAMETOOLONG.as_ptr(),
        ] {
            let s = crate::memory::cstr_as_bytes(ptr);
            assert!(s.starts_with(b"E"));
        }
    }
}
