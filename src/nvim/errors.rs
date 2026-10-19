//! Shared error messages (`src/nvim/errors.h`).
#![allow(non_upper_case_globals)]

use std::ffi::c_char;

const fn bytes_to_chars<const N: usize>(b: &[u8; N]) -> [c_char; N] {
    let mut out = [0 as c_char; N];
    let mut i = 0;
    while i < N {
        out[i] = b[i] as c_char;
        i += 1;
    }
    out
}

macro_rules! def_err {
    ($name:ident, $s:literal) => {
        #[cfg_attr(not(test), unsafe(no_mangle))]
        pub static $name: [c_char; $s.len()] = bytes_to_chars($s);
    };
}

def_err!(e_abort, b"E470: Command aborted\0");
def_err!(e_afterinit, b"E905: Cannot set this option after startup\0");
def_err!(e_api_spawn_failed, b"E903: Could not spawn API job\0");
def_err!(e_argreq, b"E471: Argument required\0");
def_err!(e_backslash, b"E10: \\ should be followed by /, ? or &\0");
def_err!(
    e_curdir,
    b"E12: Command not allowed in secure mode in current dir or tag search\0"
);
def_err!(
    e_invalid_buffer_name_str,
    b"E158: Invalid buffer name: %s\0"
);
def_err!(e_command_too_recursive, b"E169: Command too recursive\0");
def_err!(
    e_buffer_nr_invalid_buffer_number,
    b"E680: <buffer=%d>: invalid buffer number\0"
);
def_err!(e_buffer_is_not_loaded, b"E681: Buffer is not loaded\0");
def_err!(
    e_buffer_changed_unexpectedly,
    b"E787: Buffer changed unexpectedly\0"
);
def_err!(e_endif, b"E171: Missing :endif\0");
def_err!(e_endtry, b"E600: Missing :endtry\0");
def_err!(e_endwhile, b"E170: Missing :endwhile\0");
def_err!(e_endfor, b"E170: Missing :endfor\0");
def_err!(e_while, b"E588: :endwhile without :while\0");
def_err!(e_for, b"E588: :endfor without :for\0");
def_err!(e_exists, b"E13: File exists (add ! to override)\0");
def_err!(e_failed, b"E472: Command failed\0");
def_err!(e_intern2, b"E685: Internal error: %s\0");
def_err!(e_interr, b"Interrupted\0");
def_err!(e_invarg, b"E474: Invalid argument\0");
def_err!(e_invarg2, b"E475: Invalid argument: %s\0");
def_err!(e_invargval, b"E475: Invalid value for argument %s\0");
def_err!(e_invargNval, b"E475: Invalid value for argument %s: %s\0");
def_err!(e_duparg2, b"E983: Duplicate argument: %s\0");
def_err!(e_invexpr2, b"E15: Invalid expression: \"%s\"\0");
def_err!(e_invrange, b"E16: Invalid range\0");
def_err!(
    e_internal_error_in_regexp,
    b"E473: Internal error in regexp\0"
);
def_err!(e_invcmd, b"E476: Invalid command\0");
def_err!(e_isadir2, b"E17: \"%s\" is a directory\0");
def_err!(e_no_spell, b"E756: Spell checking is not possible\0");
def_err!(e_invchan, b"E900: Invalid channel id\0");
def_err!(e_invchanjob, b"E900: Invalid channel id: not a job\0");
def_err!(e_jobtblfull, b"E901: Job table is full\0");
def_err!(e_jobspawn, b"E903: Process failed to start: %s: \"%s\"\0");
def_err!(e_channotpty, b"E904: channel is not a pty\0");
def_err!(e_stdiochan2, b"E905: Couldn't open stdio channel: %s\0");
def_err!(e_invstream, b"E906: invalid stream for channel\0");
def_err!(
    e_invstreamrpc,
    b"E906: invalid stream for rpc channel, use 'rpc'\0"
);
#[cfg(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
))]
def_err!(
    e_streamkey,
    b"E5210: dict key '%s' already set for buffered stream in channel %lu\0"
);
#[cfg(not(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
)))]
def_err!(
    e_streamkey,
    b"E5210: dict key '%s' already set for buffered stream in channel %llu\0"
);
def_err!(e_libcall, b"E364: Library call failed for \"%s()\"\0");
def_err!(e_fsync, b"E667: Fsync failed: %s\0");
def_err!(e_mkdir, b"E739: Cannot create directory %s: %s\0");
def_err!(e_markinval, b"E19: Mark has invalid line number\0");
def_err!(e_marknotset, b"E20: Mark not set\0");
def_err!(
    e_modifiable,
    b"E21: Cannot make changes, 'modifiable' is off\0"
);
def_err!(e_nesting, b"E22: Scripts nested too deep\0");
def_err!(e_noalt, b"E23: No alternate file\0");
def_err!(e_noabbr, b"E24: No such abbreviation\0");
def_err!(e_nobang, b"E477: No ! allowed\0");
def_err!(e_nogroup, b"E28: No such highlight group name: %s\0");
def_err!(e_noinstext, b"E29: No inserted text yet\0");
def_err!(e_nolastcmd, b"E30: No previous command line\0");
def_err!(e_nomap, b"E31: No such mapping\0");
def_err!(e_noident, b"E349: No identifier under cursor\0");
def_err!(e_nomatch, b"E479: No match\0");
def_err!(e_nomatch2, b"E480: No match: %s\0");
def_err!(e_noname, b"E32: No file name\0");
def_err!(
    e_nopresub,
    b"E33: No previous substitute regular expression\0"
);
def_err!(e_noprev, b"E34: No previous command\0");
def_err!(e_noprevre, b"E35: No previous regular expression\0");
def_err!(e_norange, b"E481: No range allowed\0");
def_err!(e_noroom, b"E36: Not enough room\0");
def_err!(e_notmp, b"E483: Can't get temp file name\0");
def_err!(e_notopen, b"E484: Can't open file %s\0");
def_err!(e_notopen_2, b"E484: Can't open file %s: %s\0");
def_err!(e_cant_read_file_str, b"E485: Can't read file %s\0");
def_err!(e_null, b"E38: Null argument\0");
def_err!(e_number_exp, b"E39: Number expected\0");
def_err!(e_openerrf, b"E40: Can't open errorfile %s\0");
def_err!(e_outofmem, b"E41: Out of memory!\0");
def_err!(e_patnotf2, b"E486: Pattern not found: %s\0");
def_err!(e_positive, b"E487: Argument must be positive\0");
def_err!(e_prev_dir, b"E459: Cannot go back to previous directory\0");
def_err!(e_no_errors, b"E42: No Errors\0");
def_err!(e_loclist, b"E776: No location list\0");
def_err!(e_re_damg, b"E43: Damaged match string\0");
def_err!(e_re_corr, b"E44: Corrupted regexp program\0");
def_err!(
    e_readonly,
    b"E45: 'readonly' option is set (add ! to override)\0"
);
def_err!(e_letwrong, b"E734: Wrong variable type for %s=\0");
def_err!(e_illvar, b"E461: Illegal variable name: %s\0");
def_err!(e_cannot_mod, b"E995: Cannot modify existing variable\0");
def_err!(
    e_cannot_change_readonly_variable_str,
    b"E46: Cannot change read-only variable \"%.*s\"\0"
);
def_err!(e_dictreq, b"E715: Dictionary required\0");
#[cfg(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
))]
def_err!(e_blobidx, b"E979: Blob index out of range: %ld\0");
#[cfg(not(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
)))]
def_err!(e_blobidx, b"E979: Blob index out of range: %lld\0");
def_err!(e_invalblob, b"E978: Invalid operation for Blob\0");
def_err!(e_toomanyarg, b"E118: Too many arguments for function: %s\0");
def_err!(
    e_toofewarg,
    b"E119: Not enough arguments for function: %s\0"
);
def_err!(e_dictkey, b"E716: Key not present in Dictionary: \"%s\"\0");
def_err!(
    e_dictkey_len,
    b"E716: Key not present in Dictionary: \"%.*s\"\0"
);
def_err!(e_listreq, b"E714: List required\0");
def_err!(e_listblobreq, b"E897: List or Blob required\0");
def_err!(
    e_listblobarg,
    b"E899: Argument of %s must be a List or Blob\0"
);
def_err!(
    e_listdictarg,
    b"E712: Argument of %s must be a List or Dictionary\0"
);
def_err!(
    e_listdictblobarg,
    b"E896: Argument of %s must be a List, Dictionary or Blob\0"
);
def_err!(e_readerrf, b"E47: Error while reading errorfile\0");
def_err!(e_sandbox, b"E48: Not allowed in sandbox\0");
def_err!(e_secure, b"E523: Not allowed here\0");
def_err!(
    e_textlock,
    b"E565: Not allowed to change text or change window\0"
);
def_err!(e_screenmode, b"E359: Screen mode setting not supported\0");
def_err!(e_scroll, b"E49: Invalid scroll size\0");
def_err!(e_shellempty, b"E91: 'shell' option is empty\0");
def_err!(e_signdata, b"E255: Couldn't read in sign data!\0");
def_err!(e_swapclose, b"E72: Close error on swap file\0");
def_err!(e_command_too_complex, b"E74: Command too complex\0");
def_err!(e_longname, b"E75: Name too long\0");
def_err!(e_toomsbra, b"E76: Too many [\0");
def_err!(e_toomany, b"E77: Too many file names (glob not allowed)\0");
def_err!(e_trailing, b"E488: Trailing characters\0");
def_err!(e_trailing_arg, b"E488: Trailing characters: %s\0");
def_err!(e_umark, b"E78: Unknown mark\0");
def_err!(e_wildexpand, b"E79: Cannot expand wildcards\0");
def_err!(
    e_winheight,
    b"E591: 'winheight' cannot be smaller than 'winminheight'\0"
);
def_err!(
    e_winwidth,
    b"E592: 'winwidth' cannot be smaller than 'winminwidth'\0"
);
def_err!(e_write, b"E80: Error while writing\0");
def_err!(e_zerocount, b"E939: Positive count required\0");
def_err!(e_usingsid, b"E81: Using <SID> not in a script context\0");
def_err!(e_missingparen, b"E107: Missing parentheses: %s\0");
def_err!(e_empty_buffer, b"E749: Empty buffer\0");
#[cfg(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
))]
def_err!(e_nobufnr, b"E86: Buffer %ld does not exist\0");
#[cfg(not(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
)))]
def_err!(e_nobufnr, b"E86: Buffer %lld does not exist\0");
def_err!(
    e_no_write_since_last_change,
    b"E37: No write since last change\0"
);
def_err!(
    e_no_write_since_last_change_add_bang_to_override,
    b"E37: No write since last change (add ! to override)\0"
);
def_err!(
    e_no_write_since_last_change_for_buffer_nr_add_bang_to_override,
    b"E89: No write since last change for buffer %d (add ! to override)\0"
);
def_err!(e_buffer_nr_not_found, b"E92: Buffer %d not found\0");
def_err!(e_unknown_function_str, b"E117: Unknown function: %s\0");
def_err!(
    e_str_not_inside_function,
    b"E193: %s not inside a function\0"
);
def_err!(e_job_still_running, b"E948: Job still running\0");
def_err!(
    e_job_still_running_add_bang_to_end_the_job,
    b"E948: Job still running (add ! to end the job)\0"
);
def_err!(e_invalpat, b"E682: Invalid search pattern or delimiter\0");
def_err!(e_bufloaded, b"E139: File is loaded in another buffer\0");
def_err!(e_notset, b"E764: Option '%s' is not set\0");
def_err!(e_invalidreg, b"E850: Invalid register name\0");
def_err!(e_dirnotf, b"E919: Directory not found in '%s': \"%s\"\0");
def_err!(
    e_au_recursive,
    b"E952: Autocommand caused recursive behavior\0"
);
def_err!(
    e_menu_only_exists_in_another_mode,
    b"E328: Menu only exists in another mode\0"
);
def_err!(e_autocmd_close, b"E813: Cannot close autocmd window\0");
#[cfg(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
))]
def_err!(
    e_list_index_out_of_range_nr,
    b"E684: List index out of range: %ld\0"
);
#[cfg(not(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
)))]
def_err!(
    e_list_index_out_of_range_nr,
    b"E684: List index out of range: %lld\0"
);
def_err!(e_listarg, b"E686: Argument of %s must be a List\0");
def_err!(e_unsupportedoption, b"E519: Option not supported\0");
def_err!(e_fnametoolong, b"E856: Filename too long\0");
def_err!(
    e_using_float_as_string,
    b"E806: Using a Float as a String\0"
);
def_err!(
    e_cannot_edit_other_buf,
    b"E788: Not allowed to edit another buffer now\0"
);
def_err!(
    e_using_number_as_bool_nr,
    b"E1023: Using a Number as a Bool: %d\0"
);
def_err!(e_not_callable_type_str, b"E1085: Not a callable type: %s\0");
def_err!(e_auabort, b"E855: Autocommands caused command to abort\0");
def_err!(e_api_error, b"E5555: API call: %s\0");
def_err!(
    e_fast_api_disabled,
    b"E5560: %s must not be called in a fast event context\0"
);
def_err!(e_noui, b"E5768: No UI attached\0");
def_err!(
    e_floatonly,
    b"E5601: Cannot close window, only floating window would remain\0"
);
def_err!(e_floatexchange, b"E5602: Cannot exchange or rotate float\0");
def_err!(
    e_cant_find_directory_str_in_cdpath,
    b"E344: Can't find directory \"%s\" in cdpath\0"
);
def_err!(
    e_cant_find_file_str_in_path,
    b"E345: Can't find file \"%s\" in path\0"
);
def_err!(
    e_no_more_directory_str_found_in_cdpath,
    b"E346: No more directory \"%s\" found in cdpath\0"
);
def_err!(
    e_no_more_file_str_found_in_path,
    b"E347: No more file \"%s\" found in path\0"
);
def_err!(e_value_is_locked, b"E741: Value is locked\0");
def_err!(e_value_is_locked_str, b"E741: Value is locked: %.*s\0");
def_err!(e_cannot_change_value, b"E742: Cannot change value\0");
def_err!(
    e_cannot_change_value_of_str,
    b"E742: Cannot change value of %.*s\0"
);
def_err!(
    e_cannot_set_variable_in_sandbox_str,
    b"E794: Cannot set variable in the sandbox: \"%.*s\"\0"
);
def_err!(
    e_cannot_delete_variable_str,
    b"E795: Cannot delete variable %.*s\0"
);
def_err!(e_invalwindow, b"E957: Invalid window number\0");
def_err!(
    e_problem_creating_internal_diff,
    b"E960: Problem creating the internal diff\0"
);
def_err!(
    e_cannot_define_autocommands_for_all_events,
    b"E1155: Cannot define autocommands for ALL events\0"
);
def_err!(
    e_cannot_change_arglist_recursively,
    b"E1156: Cannot change the argument list recursively\0"
);
def_err!(
    e_resulting_text_too_long,
    b"E1240: Resulting text too long\0"
);
def_err!(
    e_line_number_out_of_range,
    b"E1247: Line number out of range\0"
);
def_err!(
    e_highlight_group_name_invalid_char,
    b"E5248: Invalid character in group name\0"
);
def_err!(
    e_highlight_group_name_too_long,
    b"E1249: Highlight group name too long\0"
);
def_err!(e_string_required, b"E928: String required\0");
def_err!(
    e_invalid_column_number_nr,
    b"E964: Invalid column number: %ld\0"
);
def_err!(
    e_invalid_line_number_nr,
    b"E966: Invalid line number: %ld\0"
);
def_err!(
    e_reduce_of_an_empty_str_with_no_initial_value,
    b"E998: Reduce of an empty %s with no initial value\0"
);
#[cfg(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
))]
def_err!(
    e_invalid_value_for_blob_nr,
    b"E1239: Invalid value for blob: 0xlX\0"
);
#[cfg(not(all(
    target_pointer_width = "64",
    not(target_os = "windows"),
    not(target_vendor = "apple")
)))]
def_err!(
    e_invalid_value_for_blob_nr,
    b"E1239: Invalid value for blob: 0xllX\0"
);
def_err!(
    e_stray_closing_curly_str,
    b"E1278: Stray '}' without a matching '{': %s\0"
);
def_err!(e_missing_close_curly_str, b"E1279: Missing '}': %s\0");
def_err!(
    e_cannot_change_menus_while_listing,
    b"E1310: Cannot change menus while listing\0"
);
def_err!(
    e_not_allowed_to_change_window_layout_in_this_autocmd,
    b"E1312: Not allowed to change the window layout in this autocmd\0"
);
def_err!(e_val_too_large, b"E1510: Value too large: %s\0");
def_err!(e_val_too_large_len, b"E1510: Value too large: %.*s\0");
def_err!(
    e_undobang_cannot_redo_or_move_branch,
    b"E5767: Cannot use :undo! to redo or move to a different undo branch\0"
);
def_err!(
    e_winfixbuf_cannot_go_to_buffer,
    b"E1513: Cannot switch buffer. 'winfixbuf' is enabled\0"
);
def_err!(
    e_invalid_return_type_from_findfunc,
    b"E1514: 'findfunc' did not return a List type\0"
);
def_err!(
    e_cannot_switch_to_a_closing_buffer,
    b"E1546: Cannot switch to a closing buffer\0"
);
def_err!(
    e_cannot_have_more_than_nr_diff_anchors,
    b"E1549: Cannot have more than %d diff anchors\0"
);
def_err!(
    e_failed_to_find_all_diff_anchors,
    b"E1550: Failed to find all diff anchors\0"
);
def_err!(
    e_diff_anchors_with_hidden_windows,
    b"E1562: Diff anchors cannot be used with hidden diff windows\0"
);
def_err!(
    e_leadtab_requires_tab,
    b"E1572: 'listchars' field \"leadtab\" requires \"tab\" to be specified\0"
);
def_err!(
    e_invalid_format_string_single_percent_s,
    b"E1577: Invalid format string, only one \"%s\" is allowed\0"
);
def_err!(
    e_too_many_postponed_prefixes_spell,
    b"E1578: Too many postponed prefixes and/or compound flags\0"
);
def_err!(e_trustfile, b"E5570: Cannot update trust file: %s\0");
def_err!(e_cannot_read_from_str_2, b"E282: Cannot read from \"%s\"\0");
def_err!(
    e_conflicting_configs,
    b"E5422: Conflicting configs: \"%s\" \"%s\"\0"
);
def_err!(e_unknown_option2, b"E355: Unknown option: %s\0");
def_err!(
    e_restart_failed_cmd_no_quit,
    b"E5201: Restart failed: +cmd did not quit server: %s\0"
);
def_err!(top_bot_msg, b"search hit TOP, continuing at BOTTOM\0");
def_err!(bot_top_msg, b"search hit BOTTOM, continuing at TOP\0");
def_err!(line_msg, b" line \0");

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn error_strings_are_nul_terminated_and_match_codes() {
        let abort = unsafe { CStr::from_ptr(e_abort.as_ptr()) };
        assert_eq!(abort.to_bytes(), b"E470: Command aborted");
        let invarg = unsafe { CStr::from_ptr(e_invarg.as_ptr()) };
        assert_eq!(invarg.to_bytes(), b"E474: Invalid argument");
        let lmsg = unsafe { CStr::from_ptr(line_msg.as_ptr()) };
        assert_eq!(lmsg.to_bytes(), b" line ");
    }
}
