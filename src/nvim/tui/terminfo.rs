//! Built-in fallback terminfo entries and database loader (`src/nvim/tui/terminfo.c`).

use crate::charset::{NUL, kv_transstr};
use crate::strings::vim_snprintf;
use crate::tui::termkey::driver_ti::{
    TerminfoEntry, kTermCount, kTermKeyCount, kTerminfoFuncKeyMax,
};
use std::borrow::Cow;
use std::ffi::{CStr, c_char, c_int, c_long};
use std::io::Read;
use std::ptr;

pub const kTerm_carriage_return: usize = 0;
pub const kTerm_change_scroll_region: usize = 1;
pub const kTerm_clear_screen: usize = 2;
pub const kTerm_clr_eol: usize = 3;
pub const kTerm_clr_eos: usize = 4;
pub const kTerm_cursor_address: usize = 5;
pub const kTerm_cursor_down: usize = 6;
pub const kTerm_cursor_invisible: usize = 7;
pub const kTerm_cursor_left: usize = 8;
pub const kTerm_cursor_home: usize = 9;
pub const kTerm_cursor_normal: usize = 10;
pub const kTerm_cursor_up: usize = 11;
pub const kTerm_cursor_right: usize = 12;
pub const kTerm_delete_line: usize = 13;
pub const kTerm_enter_blink_mode: usize = 14;
pub const kTerm_enter_bold_mode: usize = 15;
pub const kTerm_enter_ca_mode: usize = 16;
pub const kTerm_enter_dim_mode: usize = 17;
pub const kTerm_enter_italics_mode: usize = 18;
pub const kTerm_enter_reverse_mode: usize = 19;
pub const kTerm_enter_secure_mode: usize = 20;
pub const kTerm_enter_standout_mode: usize = 21;
pub const kTerm_enter_underline_mode: usize = 22;
pub const kTerm_erase_chars: usize = 23;
pub const kTerm_exit_attribute_mode: usize = 24;
pub const kTerm_exit_ca_mode: usize = 25;
pub const kTerm_from_status_line: usize = 26;
pub const kTerm_insert_line: usize = 27;
pub const kTerm_keypad_local: usize = 28;
pub const kTerm_keypad_xmit: usize = 29;
pub const kTerm_parm_delete_line: usize = 30;
pub const kTerm_parm_down_cursor: usize = 31;
pub const kTerm_parm_insert_line: usize = 32;
pub const kTerm_parm_left_cursor: usize = 33;
pub const kTerm_parm_right_cursor: usize = 34;
pub const kTerm_parm_up_cursor: usize = 35;
pub const kTerm_set_a_background: usize = 36;
pub const kTerm_set_a_foreground: usize = 37;
pub const kTerm_set_attributes: usize = 38;
pub const kTerm_set_lr_margin: usize = 39;
pub const kTerm_to_status_line: usize = 40;
pub const kTermExtOffset: usize = 41;
pub const kTerm_reset_cursor_style: usize = 41;
pub const kTerm_set_cursor_style: usize = 42;
pub const kTerm_enter_strikethrough_mode: usize = 43;
pub const kTerm_set_rgb_foreground: usize = 44;
pub const kTerm_set_rgb_background: usize = 45;
pub const kTerm_set_cursor_color: usize = 46;
pub const kTerm_reset_cursor_color: usize = 47;
pub const kTerm_set_underline_style: usize = 48;

pub const kTermKey_backspace: usize = 0;
pub const kTermKey_beg: usize = 1;
pub const kTermKey_btab: usize = 2;
pub const kTermKey_clear: usize = 3;
pub const kTermKey_dc: usize = 4;
pub const kTermKey_end: usize = 5;
pub const kTermKey_find: usize = 6;
pub const kTermKey_home: usize = 7;
pub const kTermKey_ic: usize = 8;
pub const kTermKey_npage: usize = 9;
pub const kTermKey_ppage: usize = 10;
pub const kTermKey_select: usize = 11;
pub const kTermKey_suspend: usize = 12;
pub const kTermKey_undo: usize = 13;
pub const kTermKey_left: usize = 14;
pub const kTermKey_right: usize = 15;

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
pub enum unibi_boolean {
    unibi_boolean_begin_ = 0,
    unibi_auto_left_margin,
    unibi_auto_right_margin,
    unibi_no_esc_ctlc,
    unibi_ceol_standout_glitch,
    unibi_eat_newline_glitch,
    unibi_erase_overstrike,
    unibi_generic_type,
    unibi_hard_copy,
    unibi_has_meta_key,
    unibi_has_status_line,
    unibi_insert_null_glitch,
    unibi_memory_above,
    unibi_memory_below,
    unibi_move_insert_mode,
    unibi_move_standout_mode,
    unibi_over_strike,
    unibi_status_line_esc_ok,
    unibi_dest_tabs_magic_smso,
    unibi_tilde_glitch,
    unibi_transparent_underline,
    unibi_xon_xoff,
    unibi_needs_xon_xoff,
    unibi_prtr_silent,
    unibi_hard_cursor,
    unibi_non_rev_rmcup,
    unibi_no_pad_char,
    unibi_non_dest_scroll_region,
    unibi_can_change,
    unibi_back_color_erase,
    unibi_hue_lightness_saturation,
    unibi_col_addr_glitch,
    unibi_cr_cancels_micro_mode,
    unibi_has_print_wheel,
    unibi_row_addr_glitch,
    unibi_semi_auto_right_margin,
    unibi_cpi_changes_res,
    unibi_lpi_changes_res,
    unibi_backspaces_with_bs,
    unibi_crt_no_scrolling,
    unibi_no_correctly_working_cr,
    unibi_gnu_has_meta_key,
    unibi_linefeed_is_newline,
    unibi_has_hardware_tabs,
    unibi_return_does_clr_eol,
    unibi_boolean_end_,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum unibi_numeric {
    unibi_numeric_begin_ = unibi_boolean::unibi_boolean_end_ as isize,
    unibi_columns,
    unibi_init_tabs,
    unibi_lines,
    unibi_lines_of_memory,
    unibi_magic_cookie_glitch,
    unibi_padding_baud_rate,
    unibi_virtual_terminal,
    unibi_width_status_line,
    unibi_num_labels,
    unibi_label_height,
    unibi_label_width,
    unibi_max_attributes,
    unibi_maximum_windows,
    unibi_max_colors,
    unibi_max_pairs,
    unibi_no_color_video,
    unibi_buffer_capacity,
    unibi_dot_vert_spacing,
    unibi_dot_horz_spacing,
    unibi_max_micro_address,
    unibi_max_micro_jump,
    unibi_micro_col_size,
    unibi_micro_line_size,
    unibi_number_of_pins,
    unibi_output_res_char,
    unibi_output_res_line,
    unibi_output_res_horz_inch,
    unibi_output_res_vert_inch,
    unibi_print_rate,
    unibi_wide_char_size,
    unibi_buttons,
    unibi_bit_image_entwining,
    unibi_bit_image_type,
    unibi_magic_cookie_glitch_ul,
    unibi_carriage_return_delay,
    unibi_new_line_delay,
    unibi_backspace_delay,
    unibi_horizontal_tab_delay,
    unibi_number_of_function_keys,
    unibi_numeric_end_,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum unibi_string {
    unibi_string_begin_ = unibi_numeric::unibi_numeric_end_ as isize,
    unibi_back_tab,
    unibi_bell,
    unibi_carriage_return,
    unibi_change_scroll_region,
    unibi_clear_all_tabs,
    unibi_clear_screen,
    unibi_clr_eol,
    unibi_clr_eos,
    unibi_column_address,
    unibi_command_character,
    unibi_cursor_address,
    unibi_cursor_down,
    unibi_cursor_home,
    unibi_cursor_invisible,
    unibi_cursor_left,
    unibi_cursor_mem_address,
    unibi_cursor_normal,
    unibi_cursor_right,
    unibi_cursor_to_ll,
    unibi_cursor_up,
    unibi_cursor_visible,
    unibi_delete_character,
    unibi_delete_line,
    unibi_dis_status_line,
    unibi_down_half_line,
    unibi_enter_alt_charset_mode,
    unibi_enter_blink_mode,
    unibi_enter_bold_mode,
    unibi_enter_ca_mode,
    unibi_enter_delete_mode,
    unibi_enter_dim_mode,
    unibi_enter_insert_mode,
    unibi_enter_secure_mode,
    unibi_enter_protected_mode,
    unibi_enter_reverse_mode,
    unibi_enter_standout_mode,
    unibi_enter_underline_mode,
    unibi_erase_chars,
    unibi_exit_alt_charset_mode,
    unibi_exit_attribute_mode,
    unibi_exit_ca_mode,
    unibi_exit_delete_mode,
    unibi_exit_insert_mode,
    unibi_exit_standout_mode,
    unibi_exit_underline_mode,
    unibi_flash_screen,
    unibi_form_feed,
    unibi_from_status_line,
    unibi_init_1string,
    unibi_init_2string,
    unibi_init_3string,
    unibi_init_file,
    unibi_insert_character,
    unibi_insert_line,
    unibi_insert_padding,
    unibi_key_backspace,
    unibi_key_catab,
    unibi_key_clear,
    unibi_key_ctab,
    unibi_key_dc,
    unibi_key_dl,
    unibi_key_down,
    unibi_key_eic,
    unibi_key_eol,
    unibi_key_eos,
    unibi_key_f0,
    unibi_key_f1,
    unibi_key_f10,
    unibi_key_f2,
    unibi_key_f3,
    unibi_key_f4,
    unibi_key_f5,
    unibi_key_f6,
    unibi_key_f7,
    unibi_key_f8,
    unibi_key_f9,
    unibi_key_home,
    unibi_key_ic,
    unibi_key_il,
    unibi_key_left,
    unibi_key_ll,
    unibi_key_npage,
    unibi_key_ppage,
    unibi_key_right,
    unibi_key_sf,
    unibi_key_sr,
    unibi_key_stab,
    unibi_key_up,
    unibi_keypad_local,
    unibi_keypad_xmit,
    unibi_lab_f0,
    unibi_lab_f1,
    unibi_lab_f10,
    unibi_lab_f2,
    unibi_lab_f3,
    unibi_lab_f4,
    unibi_lab_f5,
    unibi_lab_f6,
    unibi_lab_f7,
    unibi_lab_f8,
    unibi_lab_f9,
    unibi_meta_off,
    unibi_meta_on,
    unibi_newline,
    unibi_pad_char,
    unibi_parm_dch,
    unibi_parm_delete_line,
    unibi_parm_down_cursor,
    unibi_parm_ich,
    unibi_parm_index,
    unibi_parm_insert_line,
    unibi_parm_left_cursor,
    unibi_parm_right_cursor,
    unibi_parm_rindex,
    unibi_parm_up_cursor,
    unibi_pkey_key,
    unibi_pkey_local,
    unibi_pkey_xmit,
    unibi_print_screen,
    unibi_prtr_off,
    unibi_prtr_on,
    unibi_repeat_char,
    unibi_reset_1string,
    unibi_reset_2string,
    unibi_reset_3string,
    unibi_reset_file,
    unibi_restore_cursor,
    unibi_row_address,
    unibi_save_cursor,
    unibi_scroll_forward,
    unibi_scroll_reverse,
    unibi_set_attributes,
    unibi_set_tab,
    unibi_set_window,
    unibi_tab,
    unibi_to_status_line,
    unibi_underline_char,
    unibi_up_half_line,
    unibi_init_prog,
    unibi_key_a1,
    unibi_key_a3,
    unibi_key_b2,
    unibi_key_c1,
    unibi_key_c3,
    unibi_prtr_non,
    unibi_char_padding,
    unibi_acs_chars,
    unibi_plab_norm,
    unibi_key_btab,
    unibi_enter_xon_mode,
    unibi_exit_xon_mode,
    unibi_enter_am_mode,
    unibi_exit_am_mode,
    unibi_xon_character,
    unibi_xoff_character,
    unibi_ena_acs,
    unibi_label_on,
    unibi_label_off,
    unibi_key_beg,
    unibi_key_cancel,
    unibi_key_close,
    unibi_key_command,
    unibi_key_copy,
    unibi_key_create,
    unibi_key_end,
    unibi_key_enter,
    unibi_key_exit,
    unibi_key_find,
    unibi_key_help,
    unibi_key_mark,
    unibi_key_message,
    unibi_key_move,
    unibi_key_next,
    unibi_key_open,
    unibi_key_options,
    unibi_key_previous,
    unibi_key_print,
    unibi_key_redo,
    unibi_key_reference,
    unibi_key_refresh,
    unibi_key_replace,
    unibi_key_restart,
    unibi_key_resume,
    unibi_key_save,
    unibi_key_suspend,
    unibi_key_undo,
    unibi_key_sbeg,
    unibi_key_scancel,
    unibi_key_scommand,
    unibi_key_scopy,
    unibi_key_screate,
    unibi_key_sdc,
    unibi_key_sdl,
    unibi_key_select,
    unibi_key_send,
    unibi_key_seol,
    unibi_key_sexit,
    unibi_key_sfind,
    unibi_key_shelp,
    unibi_key_shome,
    unibi_key_sic,
    unibi_key_sleft,
    unibi_key_smessage,
    unibi_key_smove,
    unibi_key_snext,
    unibi_key_soptions,
    unibi_key_sprevious,
    unibi_key_sprint,
    unibi_key_sredo,
    unibi_key_sreplace,
    unibi_key_sright,
    unibi_key_srsume,
    unibi_key_ssave,
    unibi_key_ssuspend,
    unibi_key_sundo,
    unibi_req_for_input,
    unibi_key_f11,
    unibi_key_f12,
    unibi_key_f13,
    unibi_key_f14,
    unibi_key_f15,
    unibi_key_f16,
    unibi_key_f17,
    unibi_key_f18,
    unibi_key_f19,
    unibi_key_f20,
    unibi_key_f21,
    unibi_key_f22,
    unibi_key_f23,
    unibi_key_f24,
    unibi_key_f25,
    unibi_key_f26,
    unibi_key_f27,
    unibi_key_f28,
    unibi_key_f29,
    unibi_key_f30,
    unibi_key_f31,
    unibi_key_f32,
    unibi_key_f33,
    unibi_key_f34,
    unibi_key_f35,
    unibi_key_f36,
    unibi_key_f37,
    unibi_key_f38,
    unibi_key_f39,
    unibi_key_f40,
    unibi_key_f41,
    unibi_key_f42,
    unibi_key_f43,
    unibi_key_f44,
    unibi_key_f45,
    unibi_key_f46,
    unibi_key_f47,
    unibi_key_f48,
    unibi_key_f49,
    unibi_key_f50,
    unibi_key_f51,
    unibi_key_f52,
    unibi_key_f53,
    unibi_key_f54,
    unibi_key_f55,
    unibi_key_f56,
    unibi_key_f57,
    unibi_key_f58,
    unibi_key_f59,
    unibi_key_f60,
    unibi_key_f61,
    unibi_key_f62,
    unibi_key_f63,
    unibi_clr_bol,
    unibi_clear_margins,
    unibi_set_left_margin,
    unibi_set_right_margin,
    unibi_label_format,
    unibi_set_clock,
    unibi_display_clock,
    unibi_remove_clock,
    unibi_create_window,
    unibi_goto_window,
    unibi_hangup,
    unibi_dial_phone,
    unibi_quick_dial,
    unibi_tone,
    unibi_pulse,
    unibi_flash_hook,
    unibi_fixed_pause,
    unibi_wait_tone,
    unibi_user0,
    unibi_user1,
    unibi_user2,
    unibi_user3,
    unibi_user4,
    unibi_user5,
    unibi_user6,
    unibi_user7,
    unibi_user8,
    unibi_user9,
    unibi_orig_pair,
    unibi_orig_colors,
    unibi_initialize_color,
    unibi_initialize_pair,
    unibi_set_color_pair,
    unibi_set_foreground,
    unibi_set_background,
    unibi_change_char_pitch,
    unibi_change_line_pitch,
    unibi_change_res_horz,
    unibi_change_res_vert,
    unibi_define_char,
    unibi_enter_doublewide_mode,
    unibi_enter_draft_quality,
    unibi_enter_italics_mode,
    unibi_enter_leftward_mode,
    unibi_enter_micro_mode,
    unibi_enter_near_letter_quality,
    unibi_enter_normal_quality,
    unibi_enter_shadow_mode,
    unibi_enter_subscript_mode,
    unibi_enter_superscript_mode,
    unibi_enter_upward_mode,
    unibi_exit_doublewide_mode,
    unibi_exit_italics_mode,
    unibi_exit_leftward_mode,
    unibi_exit_micro_mode,
    unibi_exit_shadow_mode,
    unibi_exit_subscript_mode,
    unibi_exit_superscript_mode,
    unibi_exit_upward_mode,
    unibi_micro_column_address,
    unibi_micro_down,
    unibi_micro_left,
    unibi_micro_right,
    unibi_micro_row_address,
    unibi_micro_up,
    unibi_order_of_pins,
    unibi_parm_down_micro,
    unibi_parm_left_micro,
    unibi_parm_right_micro,
    unibi_parm_up_micro,
    unibi_select_char_set,
    unibi_set_bottom_margin,
    unibi_set_bottom_margin_parm,
    unibi_set_left_margin_parm,
    unibi_set_right_margin_parm,
    unibi_set_top_margin,
    unibi_set_top_margin_parm,
    unibi_start_bit_image,
    unibi_start_char_set_def,
    unibi_stop_bit_image,
    unibi_stop_char_set_def,
    unibi_subscript_characters,
    unibi_superscript_characters,
    unibi_these_cause_cr,
    unibi_zero_motion,
    unibi_char_set_names,
    unibi_key_mouse,
    unibi_mouse_info,
    unibi_req_mouse_pos,
    unibi_get_mouse,
    unibi_set_a_foreground,
    unibi_set_a_background,
    unibi_pkey_plab,
    unibi_device_type,
    unibi_code_set_init,
    unibi_set0_des_seq,
    unibi_set1_des_seq,
    unibi_set2_des_seq,
    unibi_set3_des_seq,
    unibi_set_lr_margin,
    unibi_set_tb_margin,
    unibi_bit_image_repeat,
    unibi_bit_image_newline,
    unibi_bit_image_carriage_return,
    unibi_color_names,
    unibi_define_bit_image_region,
    unibi_end_bit_image_region,
    unibi_set_color_band,
    unibi_set_page_length,
    unibi_display_pc_char,
    unibi_enter_pc_charset_mode,
    unibi_exit_pc_charset_mode,
    unibi_enter_scancode_mode,
    unibi_exit_scancode_mode,
    unibi_pc_term_options,
    unibi_scancode_escape,
    unibi_alt_scancode_esc,
    unibi_enter_horizontal_hl_mode,
    unibi_enter_left_hl_mode,
    unibi_enter_low_hl_mode,
    unibi_enter_right_hl_mode,
    unibi_enter_top_hl_mode,
    unibi_enter_vertical_hl_mode,
    unibi_set_a_attributes,
    unibi_set_pglen_inch,
    unibi_termcap_init2,
    unibi_termcap_reset,
    unibi_linefeed_if_not_lf,
    unibi_backspace_if_not_bs,
    unibi_other_non_function_keys,
    unibi_arrow_key_map,
    unibi_acs_ulcorner,
    unibi_acs_llcorner,
    unibi_acs_urcorner,
    unibi_acs_lrcorner,
    unibi_acs_ltee,
    unibi_acs_rtee,
    unibi_acs_btee,
    unibi_acs_ttee,
    unibi_acs_hline,
    unibi_acs_vline,
    unibi_acs_plus,
    unibi_memory_lock,
    unibi_memory_unlock,
    unibi_box_chars_1,
    unibi_string_end_,
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
    defs: [Option<&'static CStr>; kTermCount],
    keys: [[Option<&'static CStr>; 2]; kTermKeyCount],
    f_keys: [Option<&'static CStr>; kTerminfoFuncKeyMax],
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
            defs: [const { None }; kTermCount],
            keys: [const { [None, None] }; kTermKeyCount],
            f_keys: [const { None }; kTerminfoFuncKeyMax],
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
        defs: [None; kTermCount],
        keys: [[None; 2]; kTermKeyCount],
        f_keys: [None; kTerminfoFuncKeyMax],
    }
}

static ANSI_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(false, 8, 24, 80);
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\x1b[B");
    e.defs[kTerm_cursor_left] = Some(c"\x1b[D");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_blink_mode] = Some(c"\x1b[5m");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_secure_mode] = Some(c"\x1b[8m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_erase_chars] = Some(c"\x1b[%p1%dX");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[0;10m");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] = Some(c"\x1b[4%p1%dm");
    e.defs[kTerm_set_a_foreground] = Some(c"\x1b[3%p1%dm");
    e.defs[kTerm_set_attributes] =
        Some(c"\x1b[0;10%?%p1%t;7%;%?%p2%t;4%;%?%p3%t;7%;%?%p4%t;5%;%?%p6%t;1%;%?%p7%t;8%;%?%p9%t;11%;m");
    e.keys[kTermKey_backspace] = [Some(c"\x08"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_home] = [Some(c"\x1b[H"), None];
    e.keys[kTermKey_ic] = [Some(c"\x1b[L"), None];
    e.keys[kTermKey_left] = [Some(c"\x1b[D"), None];
    e.keys[kTermKey_right] = [Some(c"\x1b[C"), None];
    e
};

static GHOSTTY_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[2J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\n");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[?12l\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[?1049h");
    e.defs[kTerm_enter_dim_mode] = Some(c"\x1b[2m");
    e.defs[kTerm_enter_italics_mode] = Some(c"\x1b[3m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_secure_mode] = Some(c"\x1b[8m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_erase_chars] = Some(c"\x1b[%p1%dX");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b(B\x1b[m");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[?1049l");
    e.defs[kTerm_from_status_line] = Some(c"\x07");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_keypad_local] = Some(c"\x1b[?1l\x1b>");
    e.defs[kTerm_keypad_xmit] = Some(c"\x1b[?1h\x1b=");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[kTerm_set_a_foreground] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[kTerm_set_attributes] =
        Some(c"%?%p9%t\x1b(0%e\x1b(B%;\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p5%t;2%;%?%p7%t;8%;m");
    e.defs[kTerm_set_lr_margin] = Some(c"\x1b[?69h\x1b[%i%p1%d;%p2%ds");
    e.defs[kTerm_to_status_line] = Some(c"\x1b]2;");
    e.defs[kTerm_reset_cursor_style] = Some(c"\x1b[2 q");
    e.defs[kTerm_set_cursor_style] = Some(c"\x1b[%p1%d q");
    e.defs[kTerm_enter_strikethrough_mode] = Some(c"\x1b[9m");
    e.defs[kTerm_set_underline_style] = Some(c"\x1b[4:%p1%dm");
    e.keys[kTermKey_backspace] = [Some(c"\x7f"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[kTermKey_end] = [Some(c"\x1bOF"), Some(c"\x1b[1;2F")];
    e.keys[kTermKey_home] = [Some(c"\x1bOH"), Some(c"\x1b[1;2H")];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_left] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[kTermKey_right] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
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
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[2J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\n");
    e.defs[kTerm_cursor_left] = Some(c"\x1b[D");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[s\x1b[1b");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[0m");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[2b\x1b[u\r\x1b[K");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] = Some(c"\x1b[4%p1%dm");
    e.defs[kTerm_set_a_foreground] = Some(c"\x1b[3%p1%dm");
    e.keys[kTermKey_backspace] = [Some(c"\x08"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x7f"), None];
    e.keys[kTermKey_end] = [Some(c"\x1b[U"), None];
    e.keys[kTermKey_home] = [Some(c"\x1b[H"), None];
    e.keys[kTermKey_ic] = [Some(c"\x1b[L"), None];
    e.keys[kTermKey_npage] = [Some(c"\x1b[T"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[S"), None];
    e.keys[kTermKey_left] = [Some(c"\x1b[D"), Some(c"\x1bF^")];
    e.keys[kTermKey_right] = [Some(c"\x1b[C"), Some(c"\x1bF$")];
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
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\n");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_blink_mode] = Some(c"\x1b[5m");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[?1049h\x1b[22;0;0t");
    e.defs[kTerm_enter_dim_mode] = Some(c"\x1b[2m");
    e.defs[kTerm_enter_italics_mode] = Some(c"\x1b[3m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[m\x0f");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[?1049l\x1b[23;0;0t");
    e.defs[kTerm_from_status_line] = Some(c"\x07");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_keypad_local] = Some(c"\x1b[?1l\x1b>");
    e.defs[kTerm_keypad_xmit] = Some(c"\x1b[?1h\x1b=");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[kTerm_set_a_foreground] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[kTerm_set_attributes] =
        Some(c"\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;%?%p5%t;2%;m%?%p9%t^N%e\x0f%;");
    e.defs[kTerm_to_status_line] = Some(c"\x1b]2;");
    e.defs[kTerm_enter_strikethrough_mode] = Some(c"\x1b[9m");
    e.defs[kTerm_set_underline_style] = Some(c"\x1b[4:%p1%dm");
    e.keys[kTermKey_backspace] = [Some(c"\x7f"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), None];
    e.keys[kTermKey_end] = [Some(c"\x1bOF"), Some(c"\x1b[1;2F")];
    e.keys[kTermKey_home] = [Some(c"\x1bOH"), Some(c"\x1b[1;2H")];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_left] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[kTermKey_right] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
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
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\n");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l\x1b[?1c");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[?25h\x1b[?0c");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_blink_mode] = Some(c"\x1b[5m");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_dim_mode] = Some(c"\x1b[2m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_erase_chars] = Some(c"\x1b[%p1%dX");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[m\x0f");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] = Some(c"\x1b[4%p1%dm");
    e.defs[kTerm_set_a_foreground] = Some(c"\x1b[3%p1%dm");
    e.defs[kTerm_set_attributes] =
        Some(c"\x1b[0;10%?%p1%t;7%;%?%p2%t;4%;%?%p3%t;7%;%?%p4%t;5%;%?%p5%t;2%;%?%p6%t;1%;m%?%p9%t^N%e\x0f%;");
    e.keys[kTermKey_backspace] = [Some(c"\x7f"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b^I"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), None];
    e.keys[kTermKey_end] = [Some(c"\x1b[4~"), None];
    e.keys[kTermKey_home] = [Some(c"\x1b[1~"), None];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), None];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_suspend] = [Some(c"^Z"), None];
    e.keys[kTermKey_left] = [Some(c"\x1b[D"), None];
    e.keys[kTermKey_right] = [Some(c"\x1b[C"), None];
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
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\x1bD");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1bM");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_blink_mode] = Some(c"\x1b[5m");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[?1049h");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_erase_chars] = Some(c"\x1b[%p1%dX");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[m\x0f");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[?1049l");
    e.defs[kTerm_from_status_line] = Some(c"\x07");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_keypad_local] = Some(c"\x1b[?1l\x1b>");
    e.defs[kTerm_keypad_xmit] = Some(c"\x1b[?1h\x1b=");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[kTerm_set_a_foreground] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[kTerm_set_attributes] =
        Some(c"\x1b[0%?%p1%p6%|%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;m%?%p9%t^N%e\x0f%;");
    e.defs[kTerm_to_status_line] = Some(c"\x1b]0;");
    e.defs[kTerm_enter_strikethrough_mode] = Some(c"\x1b[9m");
    e.keys[kTermKey_backspace] = [Some(c"\x7f"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), None];
    e.keys[kTermKey_end] = [Some(c"\x1b[4~"), None];
    e.keys[kTermKey_home] = [Some(c"\x1b[1~"), None];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), None];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_suspend] = [Some(c"^Z"), None];
    e.keys[kTermKey_left] = [Some(c"\x1bOD"), None];
    e.keys[kTermKey_right] = [Some(c"\x1bOC"), None];
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
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[2J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\n");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_blink_mode] = Some(c"\x1b[5m");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b7\x1b[?47h");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[m\x0f");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[2J\x1b[?47l\x1b8");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_keypad_local] = Some(c"\x1b>");
    e.defs[kTerm_keypad_xmit] = Some(c"\x1b=");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[kTerm_set_a_foreground] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[kTerm_set_attributes] =
        Some(c"\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;m%?%p9%t^N%e\x0f%;");
    e.keys[kTermKey_backspace] = [Some(c"\x08"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), Some(c"\x1b[3$")];
    e.keys[kTermKey_end] = [Some(c"\x1b[8~"), Some(c"\x1b[8$")];
    e.keys[kTermKey_find] = [Some(c"\x1b[1~"), None];
    e.keys[kTermKey_home] = [Some(c"\x1b[7~"), Some(c"\x1b[7$")];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), Some(c"\x1b[2$")];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_select] = [Some(c"\x1b[4~"), None];
    e.keys[kTermKey_left] = [Some(c"\x1b[D"), Some(c"\x1b[d")];
    e.keys[kTermKey_right] = [Some(c"\x1b[C"), Some(c"\x1b[c")];
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
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\n");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[34h\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1bM");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_blink_mode] = Some(c"\x1b[5m");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[?1049h");
    e.defs[kTerm_enter_dim_mode] = Some(c"\x1b[2m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[3m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[m\x0f");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[?1049l");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_keypad_local] = Some(c"\x1b[?1l\x1b>");
    e.defs[kTerm_keypad_xmit] = Some(c"\x1b[?1h\x1b=");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[kTerm_set_a_foreground] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[kTerm_set_attributes] =
        Some(c"\x1b[0%?%p6%t;1%;%?%p1%t;3%;%?%p2%t;4%;%?%p3%t;7%;%?%p4%t;5%;%?%p5%t;2%;m%?%p9%t^N%e\x0f%;");
    e.keys[kTermKey_backspace] = [Some(c"\x7f"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), None];
    e.keys[kTermKey_end] = [Some(c"\x1b[4~"), None];
    e.keys[kTermKey_home] = [Some(c"\x1b[1~"), None];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), None];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_left] = [Some(c"\x1bOD"), None];
    e.keys[kTermKey_right] = [Some(c"\x1bOC"), None];
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

const fn set_std_63_fkeys(f_keys: &mut [Option<&'static CStr>; kTerminfoFuncKeyMax]) {
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
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[2J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\n");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_blink_mode] = Some(c"\x1b[5m");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[?1049h");
    e.defs[kTerm_enter_dim_mode] = Some(c"\x1b[2m");
    e.defs[kTerm_enter_italics_mode] = Some(c"\x1b[3m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_secure_mode] = Some(c"\x1b[8m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_erase_chars] = Some(c"\x1b[%p1%dX");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[0m");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[?1049l");
    e.defs[kTerm_from_status_line] = Some(c"\x07");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_keypad_local] = Some(c"\x1b[?1l\x1b>");
    e.defs[kTerm_keypad_xmit] = Some(c"\x1b[?1h\x1b=");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[kTerm_set_a_foreground] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[kTerm_set_attributes] =
        Some(c"%?%p9%t\x1b(0%e\x1b(B%;\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;%?%p5%t;2%;%?%p7%t;8%;m");
    e.defs[kTerm_to_status_line] = Some(c"\x1b]0;");
    e.defs[kTerm_reset_cursor_style] = Some(c"\x1b[2 q");
    e.defs[kTerm_set_cursor_style] = Some(c"\x1b[%p1%d q");
    e.defs[kTerm_enter_strikethrough_mode] = Some(c"\x1b[9m");
    e.defs[kTerm_set_cursor_color] = Some(c"\x1b]12;%p1%s\x07");
    e.keys[kTermKey_backspace] = [Some(c"\x7f"), None];
    e.keys[kTermKey_clear] = [Some(c"\x1b[3;5~"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[kTermKey_end] = [Some(c"\x1b[4~"), Some(c"\x1b[1;2F")];
    e.keys[kTermKey_home] = [Some(c"\x1b[1~"), Some(c"\x1b[1;2H")];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_left] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[kTermKey_right] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
    set_std_63_fkeys(&mut e.f_keys);
    e
};

static TMUX_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(false, 0x100, 24, 80);
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\n");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[34h\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1bM");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_blink_mode] = Some(c"\x1b[5m");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[?1049h");
    e.defs[kTerm_enter_dim_mode] = Some(c"\x1b[2m");
    e.defs[kTerm_enter_italics_mode] = Some(c"\x1b[3m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_secure_mode] = Some(c"\x1b[8m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[m\x0f");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[?1049l");
    e.defs[kTerm_from_status_line] = Some(c"\x07");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_keypad_local] = Some(c"\x1b[?1l\x1b>");
    e.defs[kTerm_keypad_xmit] = Some(c"\x1b[?1h\x1b=");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[kTerm_set_a_foreground] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[kTerm_set_attributes] =
        Some(c"\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;%?%p5%t;2%;%?%p7%t;8%;m%?%p9%t^N%e\x0f%;");
    e.defs[kTerm_to_status_line] = Some(c"\x1b]0;");
    e.defs[kTerm_reset_cursor_style] = Some(c"\x1b[2 q");
    e.defs[kTerm_set_cursor_style] = Some(c"\x1b[%p1%d q");
    e.defs[kTerm_enter_strikethrough_mode] = Some(c"\x1b[9m");
    e.defs[kTerm_set_cursor_color] = Some(c"\x1b]12;%p1%s\x07");
    e.defs[kTerm_reset_cursor_color] = Some(c"\x1b]112\x07");
    e.defs[kTerm_set_underline_style] = Some(c"\x1b[4:%p1%dm");
    e.keys[kTermKey_backspace] = [Some(c"\x7f"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[kTermKey_end] = [Some(c"\x1b[4~"), Some(c"\x1b[1;2F")];
    e.keys[kTermKey_home] = [Some(c"\x1b[1~"), Some(c"\x1b[1;2H")];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_left] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[kTermKey_right] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
    set_std_63_fkeys(&mut e.f_keys);
    e
};

static VTE_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[2J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\n");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_blink_mode] = Some(c"\x1b[5m");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[?1049h\x1b[22;0;0t");
    e.defs[kTerm_enter_dim_mode] = Some(c"\x1b[2m");
    e.defs[kTerm_enter_italics_mode] = Some(c"\x1b[3m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_secure_mode] = Some(c"\x1b[8m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_erase_chars] = Some(c"\x1b[%p1%dX");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[0m\x0f");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[?1049l\x1b[23;0;0t");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_keypad_local] = Some(c"\x1b[?1l\x1b>");
    e.defs[kTerm_keypad_xmit] = Some(c"\x1b[?1h\x1b=");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[kTerm_set_a_foreground] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[kTerm_set_attributes] =
        Some(c"\x1b[0%?%p6%t;1%;%?%p2%t;4%;%?%p4%t;5%;%?%p5%t;2%;%?%p7%t;8%;%?%p1%p3%|%t;7%;m%?%p9%t^N%e\x0f%;");
    e.defs[kTerm_reset_cursor_style] = Some(c"\x1b[1 q");
    e.defs[kTerm_set_cursor_style] = Some(c"\x1b[%p1%d q");
    e.defs[kTerm_enter_strikethrough_mode] = Some(c"\x1b[9m");
    e.defs[kTerm_set_cursor_color] = Some(c"\x1b]12;%p1%s\x07");
    e.defs[kTerm_reset_cursor_color] = Some(c"\x1b]112\x07");
    e.defs[kTerm_set_underline_style] = Some(c"\x1b[4:%p1%dm");
    e.keys[kTermKey_backspace] = [Some(c"\x7f"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[kTermKey_end] = [Some(c"\x1bOF"), Some(c"\x1b[1;2F")];
    e.keys[kTermKey_find] = [Some(c"\x1b[1~"), None];
    e.keys[kTermKey_home] = [Some(c"\x1bOH"), Some(c"\x1b[1;2H")];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_select] = [Some(c"\x1b[4~"), None];
    e.keys[kTermKey_left] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[kTermKey_right] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
    set_std_63_fkeys(&mut e.f_keys);
    e
};

static XTERM_256COLOUR_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(true, 0x100, 24, 80);
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[2J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\n");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[?12l\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_blink_mode] = Some(c"\x1b[5m");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[?1049h\x1b[22;0;0t");
    e.defs[kTerm_enter_dim_mode] = Some(c"\x1b[2m");
    e.defs[kTerm_enter_italics_mode] = Some(c"\x1b[3m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_secure_mode] = Some(c"\x1b[8m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_erase_chars] = Some(c"\x1b[%p1%dX");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b(B\x1b[m");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[?1049l\x1b[23;0;0t");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_keypad_local] = Some(c"\x1b[?1l\x1b>");
    e.defs[kTerm_keypad_xmit] = Some(c"\x1b[?1h\x1b=");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[kTerm_set_a_foreground] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[kTerm_set_attributes] =
        Some(c"%?%p9%t\x1b(0%e\x1b(B%;\x1b[0%?%p6%t;1%;%?%p5%t;2%;%?%p2%t;4%;%?%p1%p3%|%t;7%;%?%p4%t;5%;%?%p7%t;8%;m");
    e.defs[kTerm_set_lr_margin] = Some(c"\x1b[?69h\x1b[%i%p1%d;%p2%ds");
    e.defs[kTerm_reset_cursor_style] = Some(c"\x1b[2 q");
    e.defs[kTerm_set_cursor_style] = Some(c"\x1b[%p1%d q");
    e.defs[kTerm_enter_strikethrough_mode] = Some(c"\x1b[9m");
    e.defs[kTerm_set_cursor_color] = Some(c"\x1b]12;%p1%s\x07");
    e.defs[kTerm_reset_cursor_color] = Some(c"\x1b]112\x07");
    e.keys[kTermKey_backspace] = [Some(c"\x08"), None];
    e.keys[kTermKey_beg] = [Some(c"\x1bOE"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[kTermKey_end] = [Some(c"\x1bOF"), Some(c"\x1b[1;2F")];
    e.keys[kTermKey_home] = [Some(c"\x1bOH"), Some(c"\x1b[1;2H")];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_left] = [Some(c"\x1bOD"), Some(c"\x1b[1;2D")];
    e.keys[kTermKey_right] = [Some(c"\x1bOC"), Some(c"\x1b[1;2C")];
    set_std_63_fkeys(&mut e.f_keys);
    e
};

static CYGWIN_TERMINFO: BuiltinTerminfoEntry = {
    let mut e = empty_entry(false, 8, -1, -1);
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\x1b[B");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b7\x1b[?47h");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_secure_mode] = Some(c"\x1b[8m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[0;10m");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[2J\x1b[?47l\x1b8");
    e.defs[kTerm_from_status_line] = Some(c"\x07");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] = Some(c"\x1b[4%p1%dm");
    e.defs[kTerm_set_a_foreground] = Some(c"\x1b[3%p1%dm");
    e.defs[kTerm_set_attributes] =
        Some(c"\x1b[0;10%?%p1%t;7%;%?%p2%t;4%;%?%p3%t;7%;%?%p6%t;1%;%?%p7%t;8%;%?%p9%t;11%;m");
    e.defs[kTerm_to_status_line] = Some(c"\x1b];");
    e.keys[kTermKey_backspace] = [Some(c"\x08"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), None];
    e.keys[kTermKey_end] = [Some(c"\x1b[4~"), None];
    e.keys[kTermKey_home] = [Some(c"\x1b[1~"), None];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), None];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_suspend] = [Some(c"^Z"), None];
    e.keys[kTermKey_left] = [Some(c"\x1b[D"), None];
    e.keys[kTermKey_right] = [Some(c"\x1b[C"), None];
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
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\x1b[B");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b7\x1b[?47h");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[0m");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[2J\x1b[?47l\x1b8");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] = Some(c"\x1b[4%p1%dm");
    e.defs[kTerm_set_a_foreground] = Some(c"\x1b[3%p1%dm");
    e.defs[kTerm_set_attributes] = Some(c"\x1b[0%?%p1%t;7%;%?%p3%t;7%;%?%p6%t;1%;m");
    e.defs[kTerm_reset_cursor_style] = Some(c"\x1b[0 q");
    e.defs[kTerm_set_cursor_style] = Some(c"\x1b[%p1%d q");
    e.keys[kTermKey_backspace] = [Some(c"\x08"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[kTermKey_end] = [Some(c"\x1b[4~"), Some(c"\x1b[4;2~")];
    e.keys[kTermKey_home] = [Some(c"\x1b[1~"), Some(c"\x1b[1;2~")];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_suspend] = [Some(c"^Z"), None];
    e.keys[kTermKey_left] = [Some(c"\x1b[D"), Some(c"\x1b[1;2D")];
    e.keys[kTermKey_right] = [Some(c"\x1b[C"), Some(c"\x1b[1;2C")];
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
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[2J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\x1b[B");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[?1049h");
    e.defs[kTerm_enter_italics_mode] = Some(c"\x1b[3m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_erase_chars] = Some(c"\x1b[%p1%dX");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[0m");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[?1049l");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] = Some(c"\x1b[48;5;%p1%dm");
    e.defs[kTerm_set_a_foreground] = Some(c"\x1b[38;5;%p1%dm");
    e.defs[kTerm_set_attributes] = Some(c"\x1b[0%?%p1%p3%|%t;7%;%?%p2%t;4%;%?%p6%t;1%;m");
    e.defs[kTerm_reset_cursor_style] = Some(c"\x1b[2 q");
    e.defs[kTerm_set_cursor_style] = Some(c"\x1b[%p1%d q");
    e.keys[kTermKey_backspace] = [Some(c"\x08"), None];
    e.keys[kTermKey_beg] = [Some(c"\x1bOE"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[kTermKey_end] = [Some(c"\x1b[4~"), Some(c"\x1b[4;2~")];
    e.keys[kTermKey_home] = [Some(c"\x1b[1~"), Some(c"\x1b[1;2~")];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_left] = [Some(c"\x1b[D"), Some(c"\x1b[1;2D")];
    e.keys[kTermKey_right] = [Some(c"\x1b[C"), Some(c"\x1b[1;2C")];
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
    e.defs[kTerm_carriage_return] = Some(c"\r");
    e.defs[kTerm_change_scroll_region] = Some(c"\x1b[%i%p1%d;%p2%dr");
    e.defs[kTerm_clear_screen] = Some(c"\x1b[H\x1b[2J");
    e.defs[kTerm_clr_eol] = Some(c"\x1b[K");
    e.defs[kTerm_clr_eos] = Some(c"\x1b[J");
    e.defs[kTerm_cursor_address] = Some(c"\x1b[%i%p1%d;%p2%dH");
    e.defs[kTerm_cursor_down] = Some(c"\x1b[B");
    e.defs[kTerm_cursor_invisible] = Some(c"\x1b[?25l");
    e.defs[kTerm_cursor_left] = Some(c"\x08");
    e.defs[kTerm_cursor_home] = Some(c"\x1b[H");
    e.defs[kTerm_cursor_normal] = Some(c"\x1b[?12l\x1b[?25h");
    e.defs[kTerm_cursor_up] = Some(c"\x1b[A");
    e.defs[kTerm_cursor_right] = Some(c"\x1b[C");
    e.defs[kTerm_delete_line] = Some(c"\x1b[M");
    e.defs[kTerm_enter_bold_mode] = Some(c"\x1b[1m");
    e.defs[kTerm_enter_ca_mode] = Some(c"\x1b[?1049h");
    e.defs[kTerm_enter_italics_mode] = Some(c"\x1b[3m");
    e.defs[kTerm_enter_reverse_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_standout_mode] = Some(c"\x1b[7m");
    e.defs[kTerm_enter_underline_mode] = Some(c"\x1b[4m");
    e.defs[kTerm_erase_chars] = Some(c"\x1b[%p1%dX");
    e.defs[kTerm_exit_attribute_mode] = Some(c"\x1b[0m");
    e.defs[kTerm_exit_ca_mode] = Some(c"\x1b[?1049l");
    e.defs[kTerm_from_status_line] = Some(c"\x07");
    e.defs[kTerm_insert_line] = Some(c"\x1b[L");
    e.defs[kTerm_parm_delete_line] = Some(c"\x1b[%p1%dM");
    e.defs[kTerm_parm_down_cursor] = Some(c"\x1b[%p1%dB");
    e.defs[kTerm_parm_insert_line] = Some(c"\x1b[%p1%dL");
    e.defs[kTerm_parm_left_cursor] = Some(c"\x1b[%p1%dD");
    e.defs[kTerm_parm_right_cursor] = Some(c"\x1b[%p1%dC");
    e.defs[kTerm_parm_up_cursor] = Some(c"\x1b[%p1%dA");
    e.defs[kTerm_set_a_background] =
        Some(c"\x1b[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m");
    e.defs[kTerm_set_a_foreground] =
        Some(c"\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m");
    e.defs[kTerm_set_attributes] =
        Some(c"\x1b[0%?%p1%p3%|%t;7%;%?%p2%t;4%;%?%p6%t;1%;%?%p9%t;9%;m");
    e.defs[kTerm_to_status_line] = Some(c"\x1b]0;");
    e.defs[kTerm_reset_cursor_style] = Some(c"\x1b[2 q");
    e.defs[kTerm_set_cursor_style] = Some(c"\x1b[%p1%d q");
    e.defs[kTerm_enter_strikethrough_mode] = Some(c"\x1b[9m");
    e.keys[kTermKey_backspace] = [Some(c"\x08"), None];
    e.keys[kTermKey_beg] = [Some(c"\x1bOE"), None];
    e.keys[kTermKey_btab] = [Some(c"\x1b[Z"), None];
    e.keys[kTermKey_dc] = [Some(c"\x1b[3~"), Some(c"\x1b[3;2~")];
    e.keys[kTermKey_end] = [Some(c"\x1b[4~"), Some(c"\x1b[4;2~")];
    e.keys[kTermKey_home] = [Some(c"\x1b[1~"), Some(c"\x1b[1;2~")];
    e.keys[kTermKey_ic] = [Some(c"\x1b[2~"), Some(c"\x1b[2;2~")];
    e.keys[kTermKey_npage] = [Some(c"\x1b[6~"), None];
    e.keys[kTermKey_ppage] = [Some(c"\x1b[5~"), None];
    e.keys[kTermKey_left] = [Some(c"\x1b[D"), Some(c"\x1b[1;2D")];
    e.keys[kTermKey_right] = [Some(c"\x1b[C"), Some(c"\x1b[1;2C")];
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

static UNI_IDS: [unibi_string; 41] = [
    unibi_string::unibi_carriage_return,
    unibi_string::unibi_change_scroll_region,
    unibi_string::unibi_clear_screen,
    unibi_string::unibi_clr_eol,
    unibi_string::unibi_clr_eos,
    unibi_string::unibi_cursor_address,
    unibi_string::unibi_cursor_down,
    unibi_string::unibi_cursor_invisible,
    unibi_string::unibi_cursor_left,
    unibi_string::unibi_cursor_home,
    unibi_string::unibi_cursor_normal,
    unibi_string::unibi_cursor_up,
    unibi_string::unibi_cursor_right,
    unibi_string::unibi_delete_line,
    unibi_string::unibi_enter_blink_mode,
    unibi_string::unibi_enter_bold_mode,
    unibi_string::unibi_enter_ca_mode,
    unibi_string::unibi_enter_dim_mode,
    unibi_string::unibi_enter_italics_mode,
    unibi_string::unibi_enter_reverse_mode,
    unibi_string::unibi_enter_secure_mode,
    unibi_string::unibi_enter_standout_mode,
    unibi_string::unibi_enter_underline_mode,
    unibi_string::unibi_erase_chars,
    unibi_string::unibi_exit_attribute_mode,
    unibi_string::unibi_exit_ca_mode,
    unibi_string::unibi_from_status_line,
    unibi_string::unibi_insert_line,
    unibi_string::unibi_keypad_local,
    unibi_string::unibi_keypad_xmit,
    unibi_string::unibi_parm_delete_line,
    unibi_string::unibi_parm_down_cursor,
    unibi_string::unibi_parm_insert_line,
    unibi_string::unibi_parm_left_cursor,
    unibi_string::unibi_parm_right_cursor,
    unibi_string::unibi_parm_up_cursor,
    unibi_string::unibi_set_a_background,
    unibi_string::unibi_set_a_foreground,
    unibi_string::unibi_set_attributes,
    unibi_string::unibi_set_lr_margin,
    unibi_string::unibi_to_status_line,
];

static UNI_EXT: [&CStr; 8] = [
    c"Se", c"Ss", c"smxx", c"setrgbf", c"setrgbb", c"Cs", c"Cr", c"Smulx",
];

static UNI_KEYS: [[unibi_string; 2]; kTermKeyCount] = [
    [
        unibi_string::unibi_key_backspace,
        unibi_string::unibi_string_begin_,
    ],
    [unibi_string::unibi_key_beg, unibi_string::unibi_key_sbeg],
    [
        unibi_string::unibi_key_btab,
        unibi_string::unibi_string_begin_,
    ],
    [
        unibi_string::unibi_key_clear,
        unibi_string::unibi_string_begin_,
    ],
    [unibi_string::unibi_key_dc, unibi_string::unibi_key_sdc],
    [unibi_string::unibi_key_end, unibi_string::unibi_key_send],
    [unibi_string::unibi_key_find, unibi_string::unibi_key_sfind],
    [unibi_string::unibi_key_home, unibi_string::unibi_key_shome],
    [unibi_string::unibi_key_ic, unibi_string::unibi_key_sic],
    [
        unibi_string::unibi_key_npage,
        unibi_string::unibi_string_begin_,
    ],
    [
        unibi_string::unibi_key_ppage,
        unibi_string::unibi_string_begin_,
    ],
    [
        unibi_string::unibi_key_select,
        unibi_string::unibi_string_begin_,
    ],
    [
        unibi_string::unibi_key_suspend,
        unibi_string::unibi_key_ssuspend,
    ],
    [unibi_string::unibi_key_undo, unibi_string::unibi_key_sundo],
    [unibi_string::unibi_key_left, unibi_string::unibi_key_sleft],
    [
        unibi_string::unibi_key_right,
        unibi_string::unibi_key_sright,
    ],
];

static UNI_FKEYS: [unibi_string; kTerminfoFuncKeyMax] = [
    unibi_string::unibi_key_f1,
    unibi_string::unibi_key_f2,
    unibi_string::unibi_key_f3,
    unibi_string::unibi_key_f4,
    unibi_string::unibi_key_f5,
    unibi_string::unibi_key_f6,
    unibi_string::unibi_key_f7,
    unibi_string::unibi_key_f8,
    unibi_string::unibi_key_f9,
    unibi_string::unibi_key_f10,
    unibi_string::unibi_key_f11,
    unibi_string::unibi_key_f12,
    unibi_string::unibi_key_f13,
    unibi_string::unibi_key_f14,
    unibi_string::unibi_key_f15,
    unibi_string::unibi_key_f16,
    unibi_string::unibi_key_f17,
    unibi_string::unibi_key_f18,
    unibi_string::unibi_key_f19,
    unibi_string::unibi_key_f20,
    unibi_string::unibi_key_f21,
    unibi_string::unibi_key_f22,
    unibi_string::unibi_key_f23,
    unibi_string::unibi_key_f24,
    unibi_string::unibi_key_f25,
    unibi_string::unibi_key_f26,
    unibi_string::unibi_key_f27,
    unibi_string::unibi_key_f28,
    unibi_string::unibi_key_f29,
    unibi_string::unibi_key_f30,
    unibi_string::unibi_key_f31,
    unibi_string::unibi_key_f32,
    unibi_string::unibi_key_f33,
    unibi_string::unibi_key_f34,
    unibi_string::unibi_key_f35,
    unibi_string::unibi_key_f36,
    unibi_string::unibi_key_f37,
    unibi_string::unibi_key_f38,
    unibi_string::unibi_key_f39,
    unibi_string::unibi_key_f40,
    unibi_string::unibi_key_f41,
    unibi_string::unibi_key_f42,
    unibi_string::unibi_key_f43,
    unibi_string::unibi_key_f44,
    unibi_string::unibi_key_f45,
    unibi_string::unibi_key_f46,
    unibi_string::unibi_key_f47,
    unibi_string::unibi_key_f48,
    unibi_string::unibi_key_f49,
    unibi_string::unibi_key_f50,
    unibi_string::unibi_key_f51,
    unibi_string::unibi_key_f52,
    unibi_string::unibi_key_f53,
    unibi_string::unibi_key_f54,
    unibi_string::unibi_key_f55,
    unibi_string::unibi_key_f56,
    unibi_string::unibi_key_f57,
    unibi_string::unibi_key_f58,
    unibi_string::unibi_key_f59,
    unibi_string::unibi_key_f60,
    unibi_string::unibi_key_f61,
    unibi_string::unibi_key_f62,
    unibi_string::unibi_key_f63,
];

static STRING_NAMES: [&str; kTermCount] = [
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

static KEY_NAMES: [&str; kTermKeyCount] = [
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

static FKEY_NAMES: [&str; kTerminfoFuncKeyMax] = [
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
const UNIBI_BOOL_COUNT: usize = (unibi_boolean::unibi_boolean_end_ as usize)
    - (unibi_boolean::unibi_boolean_begin_ as usize)
    - 1;
const UNIBI_NUM_COUNT: usize = (unibi_numeric::unibi_numeric_end_ as usize)
    - (unibi_numeric::unibi_numeric_begin_ as usize)
    - 1;
const UNIBI_STR_COUNT: usize =
    (unibi_string::unibi_string_end_ as usize) - (unibi_string::unibi_string_begin_ as usize) - 1;

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
    fn get_bool(&self, v: unibi_boolean) -> bool {
        let idx = (v as usize).wrapping_sub((unibi_boolean::unibi_boolean_begin_ as usize) + 1);
        self.bools.get(idx).copied().unwrap_or(false)
    }

    #[inline]
    fn get_num(&self, v: unibi_numeric) -> c_int {
        let idx = (v as usize).wrapping_sub((unibi_numeric::unibi_numeric_begin_ as usize) + 1);
        self.nums.get(idx).copied().unwrap_or(-1)
    }

    #[inline]
    fn get_str(&self, v: unibi_string) -> *const c_char {
        let idx = (v as usize).wrapping_sub((unibi_string::unibi_string_begin_ as usize) + 1);
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
    unsafe {
        crate::os::env::os_getenv_cstr(name.as_ptr()).map(|c| c.to_string_lossy().into_owned())
    }
}

unsafe fn unibi_from_term(termname: *const c_char) -> Option<UnibiTerm> {
    if termname.is_null() {
        return None;
    }
    let term_bytes = unsafe { CStr::from_ptr(termname).to_bytes() };
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

pub unsafe fn terminfo_is_term_family(term: *const c_char, family: *const c_char) -> bool {
    if term.is_null() {
        return false;
    }
    unsafe {
        let term_bytes = CStr::from_ptr(term).to_bytes();
        let fam_bytes = CStr::from_ptr(family).to_bytes();
        let tlen = term_bytes.len();
        let flen = fam_bytes.len();
        tlen >= flen
            && &term_bytes[..flen] == fam_bytes
            && (tlen == flen || term_bytes[flen] == b'-' || term_bytes[flen] == b'.')
    }
}

pub unsafe fn terminfo_is_bsd_console(term: *const c_char) -> bool {
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
                && os_env_exists(c"XTERM_VERSION".as_ptr(), true);
        }
    }
    let _ = term;
    false
}

pub unsafe fn terminfo_from_builtin(
    term: *const c_char,
    termname: &mut Option<std::ffi::CString>,
) -> TerminfoEntry {
    unsafe {
        if (crate::memory::cstr_as_opt_bytes(term) == Some(b"ghostty")) || (crate::memory::cstr_as_opt_bytes(term) == Some(b"xterm-ghostty")) {
            *termname = Some(c"ghostty".to_owned());
            GHOSTTY_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"xterm".as_ptr()) {
            *termname = Some(c"xterm".to_owned());
            XTERM_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"screen".as_ptr()) {
            *termname = Some(c"screen".to_owned());
            SCREEN_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"tmux".as_ptr()) {
            *termname = Some(c"tmux".to_owned());
            TMUX_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"rxvt".as_ptr()) {
            *termname = Some(c"rxvt".to_owned());
            RXVT_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"putty".as_ptr()) {
            *termname = Some(c"putty".to_owned());
            PUTTY_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"linux".as_ptr()) {
            *termname = Some(c"linux".to_owned());
            LINUX_16COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"interix".as_ptr()) {
            *termname = Some(c"interix".to_owned());
            INTERIX_8COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"iterm".as_ptr())
            || terminfo_is_term_family(term, c"iterm2".as_ptr())
            || terminfo_is_term_family(term, c"iTerm.app".as_ptr())
            || terminfo_is_term_family(term, c"iTerm2.app".as_ptr())
        {
            *termname = Some(c"iterm".to_owned());
            ITERM_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"st".as_ptr()) {
            *termname = Some(c"st".to_owned());
            ST_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"gnome".as_ptr())
            || terminfo_is_term_family(term, c"vte".as_ptr())
        {
            *termname = Some(c"vte".to_owned());
            VTE_256COLOUR_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"cygwin".as_ptr()) {
            *termname = Some(c"cygwin".to_owned());
            CYGWIN_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"win32con".as_ptr()) {
            *termname = Some(c"win32con".to_owned());
            WIN32CON_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"conemu".as_ptr()) {
            *termname = Some(c"conemu".to_owned());
            CONEMU_TERMINFO.to_entry()
        } else if terminfo_is_term_family(term, c"vtpcon".as_ptr()) {
            *termname = Some(c"vtpcon".to_owned());
            VTPCON_TERMINFO.to_entry()
        } else {
            *termname = Some(c"ansi".to_owned());
            ANSI_TERMINFO.to_entry()
        }
    }
}

pub unsafe fn terminfo_from_database(ti: *mut TerminfoEntry, termname: *const c_char) -> bool {
    unsafe {
        let Some(ut) = unibi_from_term(termname) else {
            return false;
        };

        (*ti).back_color_erase = ut.get_bool(unibi_boolean::unibi_back_color_erase);
        (*ti).max_colors = ut.get_num(unibi_numeric::unibi_max_colors);
        (*ti).lines = ut.get_num(unibi_numeric::unibi_lines);
        (*ti).columns = ut.get_num(unibi_numeric::unibi_columns);

        (*ti).Tc = false;
        (*ti).RGB = false;
        (*ti).Su = false;
        let ext_bool_count = ut.count_ext_bool();
        for i in 0..ext_bool_count {
            let n = ut.get_ext_bool_name(i);
            if !n.is_null() {
                let bytes = CStr::from_ptr(n).to_bytes();
                if bytes == b"Tc" {
                    (*ti).Tc = true;
                } else if bytes == b"RGB" {
                    (*ti).RGB = true;
                } else if bytes == b"Su" {
                    (*ti).Su = true;
                }
            }
        }

        for (i, &uni_id) in UNI_IDS.iter().enumerate() {
            let val = ut.get_str(uni_id);
            (*ti).defs[i] = if !val.is_null() {
                Some(Cow::Owned(CStr::from_ptr(val).to_owned()))
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
                    (*ti).defs[kTermExtOffset + i] = if !data.is_null() {
                        Some(Cow::Owned(CStr::from_ptr(data).to_owned()))
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
                (*ti).keys[i][0] = Some(Cow::Owned(CStr::from_ptr(val).to_owned()));
                if pair[1] != unibi_string::unibi_string_begin_ {
                    let sval = ut.get_str(pair[1]);
                    (*ti).keys[i][1] = if !sval.is_null() {
                        Some(Cow::Owned(CStr::from_ptr(sval).to_owned()))
                    } else {
                        None
                    };
                }
            }
        }

        for (i, &fkey_id) in UNI_FKEYS.iter().enumerate() {
            let val = ut.get_str(fkey_id);
            (*ti).f_keys[i] = if !val.is_null() {
                Some(Cow::Owned(CStr::from_ptr(val).to_owned()))
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
    unsafe {
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

unsafe fn pop(num: *mut c_long, string: Option<&mut Option<[c_char; 8]>>, stack: &mut TPSTACK) -> c_int {
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

unsafe fn ochar(buf: *mut *mut c_char, buf_end: *const c_char, mut c: c_int) -> bool {
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

unsafe fn onum(
    buf: *mut *mut c_char,
    buf_end: *const c_char,
    fmt: *const c_char,
    num: c_int,
    mut len: usize,
) -> bool {
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

pub unsafe fn terminfo_fmt(
    buf_start: *mut c_char,
    buf_end: *mut c_char,
    mut str_: *const c_char,
    params: *mut TPVAR,
) -> usize {
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
                        let mut l = CStr::from_ptr(ostr_ptr).to_bytes().len();
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
                        CStr::from_ptr(s_arr.as_ptr()).to_bytes().len()
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
                    let sc = *str_ as u8;
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
                    let sc = *str_ as u8;
                    if sc.is_ascii_lowercase() {
                        dnums[(sc - b'a') as usize] = val;
                    } else if sc.is_ascii_uppercase() {
                        snums[(sc - b'A') as usize] = val;
                    }
                }
                b'g' => {
                    let sc = *str_ as u8;
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
                    let ch = *str_ as u8;
                    str_ = str_.add(1);
                    if push(c_long::from(ch), None, &mut stack) != 0 {
                        return 0;
                    }
                    while *str_ != NUL && *str_ != b'\'' as c_char {
                        str_ = str_.add(1);
                    }
                    if *str_ == b'\'' as c_char {
                        str_ = str_.add(1);
                    }
                }
                b'{' => {
                    val = 0;
                    while (*str_ as u8).is_ascii_digit() {
                        val = (val * 10) + c_long::from((*str_ as u8) - b'0');
                        str_ = str_.add(1);
                    }
                    if push(val, None, &mut stack) != 0 {
                        return 0;
                    }
                    while *str_ != NUL && *str_ != b'}' as c_char {
                        str_ = str_.add(1);
                    }
                    if *str_ == b'}' as c_char {
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
                        while *str_ != NUL {
                            if *str_ != b'%' as c_char {
                                str_ = str_.add(1);
                                continue;
                            }
                            str_ = str_.add(1);
                            if *str_ == b'?' as c_char {
                                l += 1;
                            } else if *str_ == b';' as c_char {
                                if l > 0 {
                                    l -= 1;
                                } else {
                                    str_ = str_.add(1);
                                    break;
                                }
                            } else if *str_ == b'e' as c_char && l == 0 {
                                str_ = str_.add(1);
                                break;
                            }
                            if *str_ != NUL {
                                str_ = str_.add(1);
                            }
                        }
                    }
                }
                b'e' => {
                    let mut l: usize = 0;
                    while *str_ != NUL {
                        if *str_ != b'%' as c_char {
                            str_ = str_.add(1);
                            continue;
                        }
                        str_ = str_.add(1);
                        if *str_ == b'?' as c_char {
                            l += 1;
                        } else if *str_ == b';' as c_char {
                            if l > 0 {
                                l -= 1;
                            } else {
                                str_ = str_.add(1);
                                break;
                            }
                        }
                        if *str_ != NUL {
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
        assert!(ut.get_bool(unibi_boolean::unibi_auto_left_margin));
        assert!(!ut.get_bool(unibi_boolean::unibi_back_color_erase));
        assert_eq!(ut.get_num(unibi_numeric::unibi_columns), 80);
        assert_eq!(ut.get_num(unibi_numeric::unibi_max_colors), -1);
        assert!(ut.get_str(unibi_string::unibi_back_tab).is_null());
        unsafe {
            assert_eq!(
                CStr::from_ptr(ut.get_str(unibi_string::unibi_carriage_return)).to_bytes(),
                b"\r"
            );
            assert_eq!(ut.count_ext_bool(), 1);
            assert_eq!(CStr::from_ptr(ut.get_ext_bool_name(0)).to_bytes(), b"Tc");
            assert_eq!(ut.count_ext_str(), 1);
            assert_eq!(CStr::from_ptr(ut.get_ext_str_name(0)).to_bytes(), b"Se");
            assert_eq!(CStr::from_ptr(ut.get_ext_str(0)).to_bytes(), b"\x1b[0 q");
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
