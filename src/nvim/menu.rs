//! Code for menus, used for the GUI and `'wildmenu'` (`src/nvim/menu.c`).

use crate::autocmd::{EVENT_MENUPOPUP, HLF_8, apply_autocmds};
use crate::charset::{CTRL_V_CONST, ascii_isdigit, ascii_iswhite, getdigits_int, skipwhite};
use crate::cursor::{CursorPos, check_cursor, gchar_cursor};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, TvDict, TvList, TypVal, VarType, tv_dict_add_bool, tv_dict_add_dict,
    tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_add_str_len, tv_dict_alloc,
    tv_dict_alloc_ret, tv_get_string_chk, tv_list_alloc, tv_list_append_dict,
    tv_list_append_string,
};
use crate::eval::vars::del_menutrans_vars;
use crate::ex_docmd::{
    CTRL_C_CONST, CTRL_G, CTRL_O, Exarg, ends_excmd, exec_normal_cmd, restore_current_state,
    save_current_state,
};
use crate::globals::*;
use crate::input::{REMAP_SCRIPT, Tasave, ins_typebuf};
use crate::keycodes::{REPTERM_DO_LT, replace_termcodes};
use crate::mbyte::{utf_char2bytes, utfc_ptr2len};
use crate::memory::xstrlcpy;
use crate::message::{
    emsg, msg_outnum, msg_outtrans, msg_outtrans_special, msg_putchar, msg_puts, msg_puts_hl,
    msg_puts_title, semsg, str2special_save,
};
use crate::option_vars::*;
use crate::os::env::ExpandCtx;
use crate::popupmenu::pum_show_popupmenu;
use crate::state::{
    MODE_ASKMORE, MODE_CMDLINE, MODE_HITRETURN, MODE_INSERT, MODE_LANGMAP, MODE_NORMAL,
    MODE_TERMINAL, MODE_VISUAL, get_real_state,
};
use crate::strings::{gettext, vim_strchr};
use crate::ui::ui_call_update_menu;
use std::ffi::{CStr, c_char};
use std::ptr;

const CTRL_BSL_CONST: c_char = 28;

const MENUDEPTH: usize = 10;
const TBUFFER_LEN: usize = 256;

const MENU_INDEX_INVALID: i32 = -1;
const MENU_INDEX_NORMAL: i32 = 0;
const MENU_INDEX_VISUAL: i32 = 1;
const MENU_INDEX_SELECT: i32 = 2;
const MENU_INDEX_OP_PENDING: i32 = 3;
const MENU_INDEX_INSERT: i32 = 4;
const MENU_INDEX_CMDLINE: i32 = 5;
const MENU_INDEX_TERMINAL: i32 = 6;
pub const MENU_MODES: usize = 8;

const MENU_NORMAL_MODE: i32 = 1 << MENU_INDEX_NORMAL;
const MENU_VISUAL_MODE: i32 = 1 << MENU_INDEX_VISUAL;
const MENU_SELECT_MODE: i32 = 1 << MENU_INDEX_SELECT;
const MENU_OP_PENDING_MODE: i32 = 1 << MENU_INDEX_OP_PENDING;
const MENU_INSERT_MODE: i32 = 1 << MENU_INDEX_INSERT;
const MENU_CMDLINE_MODE: i32 = 1 << MENU_INDEX_CMDLINE;
const MENU_TERMINAL_MODE: i32 = 1 << MENU_INDEX_TERMINAL;
const MENU_TIP_MODE: i32 = 1 << MENU_INDEX_TIP;

const MNU_HIDDEN_CHAR: c_char = b']' as c_char;

const MENU_MODE_CHARS: [&CStr; MENU_MODES] = [c"n", c"v", c"s", c"o", c"i", c"c", c"tl", c"t"];

const E_NOTSUBMENU: &CStr = c"E327: Part of menu-item path is not sub-menu";
const E_NOMENU: &CStr = c"E329: No menu \"%s\"";

#[repr(C)]
pub struct Vimmenu {
    pub modes: i32,
    pub enabled: i32,
    pub name: std::ffi::CString,
    pub dname: std::ffi::CString,
    pub en_name: Option<std::ffi::CString>,
    pub en_dname: Option<std::ffi::CString>,
    pub mnemonic: i32,
    pub actext: Option<std::ffi::CString>,
    pub priority: i32,
    pub strings: [Option<std::ffi::CString>; MENU_MODES],
    pub noremap: [i32; MENU_MODES],
    pub silent: [bool; MENU_MODES],
    pub children: Vec<Vimmenu>,
}

impl Vimmenu {
    pub fn new() -> Self {
        Self {
            modes: 0,
            enabled: 0,
            name: std::ffi::CString::default(),
            dname: std::ffi::CString::default(),
            en_name: None,
            en_dname: None,
            mnemonic: 0,
            actext: None,
            priority: 0,
            strings: [const { None }; MENU_MODES],
            noremap: [0; MENU_MODES],
            silent: [false; MENU_MODES],
            children: Vec::new(),
        }
    }
}

impl Default for Vimmenu {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
struct Menutrans {
    from: std::ffi::CString,
    from_noamp: std::ffi::CString,
    to: std::ffi::CString,
}

use crate::change::{MB_MAXCHAR, TAB};
use crate::charset::NUL;
use crate::cmdexpand::{
    EXPAND_MENUNAMES, EXPAND_MENUS, EXPAND_NOTHING, EXPAND_UNSUCCESSFUL, HLF_D,
};
use crate::digraph::{K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST};
use crate::eval::K_LIST_LEN_MAY_KNOW_CONST;
use crate::eval::funcs::{MENU_ALL_MODES, MENU_INDEX_TIP};
use crate::extmark::{K_FALSE, K_NONE, K_TRUE};
use crate::mapping::{REMAP_NONE, REMAP_YES};
use crate::regexp::{MAXCOL, NUMBUFLEN};

#[repr(C)]
#[derive(Clone)]
pub struct SaveState {
    pub save_msg_scroll: i32,
    pub save_restart_edit: i32,
    pub save_msg_didout: bool,
    pub save_state: i32,
    pub save_finish_op: bool,
    pub save_opcount: i32,
    pub save_reg_executing: i32,
    pub save_pending_end_reg_executing: bool,
    pub tabuf: Tasave,
}

impl SaveState {
    pub const fn new() -> Self {
        Self {
            save_msg_scroll: 0,
            save_restart_edit: 0,
            save_msg_didout: false,
            save_state: 0,
            save_finish_op: false,
            save_opcount: 0,
            save_reg_executing: 0,
            save_pending_end_reg_executing: false,
            tabuf: Tasave::new(),
        }
    }
}

struct MenuModuleState {
    menus_locked: i32,
    expand_menu: *const Vec<Vimmenu>,
    expand_modes: i32,
    expand_emenu: bool,
    get_menu_name_idx: usize,
    get_menu_name_advance: bool,
    get_menu_names_idx: usize,
    get_menu_names_advance: bool,
    get_menu_names_tbuffer: [c_char; TBUFFER_LEN],
    menutrans_ga: Vec<Menutrans>,
}

impl MenuModuleState {
    const fn new() -> Self {
        Self {
            menus_locked: 0,
            expand_menu: ptr::null(),
            expand_modes: 0,
            expand_emenu: false,
            get_menu_name_idx: 0,
            get_menu_name_advance: false,
            get_menu_names_idx: 0,
            get_menu_names_advance: false,
            get_menu_names_tbuffer: [0; TBUFFER_LEN],
            menutrans_ga: Vec::new(),
        }
    }
}

static MENU_STATE: GlobalState<MenuModuleState> = GlobalState::new(MenuModuleState::new());

#[inline]
fn menu_st() -> &'static mut MenuModuleState {
    unsafe { &mut *MENU_STATE.as_ptr() }
}

use crate::eval::typval::tv_dict_len;

/// Return true if `name` is a window toolbar menu name (`menu_is_winbar`).
fn menu_is_winbar(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    crate::memory::cstr_prefix_bytes(name, 6) == &b"WinBar"[..6]
}

fn get_root_menu(_name: *const c_char) -> *mut Vec<Vimmenu> {
    &raw mut g_st().root_menu
}

/// If `menus_locked` is set then give an error and return true (`is_menus_locked`).
fn is_menus_locked() -> bool {
    if menu_st().menus_locked > 0 {
        emsg(gettext(E_CANNOT_CHANGE_MENUS_WHILE_LISTING));
        return true;
    }
    false
}

/// Do the `:menu` command and relatives (`ex_menu`).
pub fn ex_menu(eap: &mut Exarg) {
    unsafe {
        let mut noremap: i32 = 0;
        let mut silent = false;
        let mut unmenu = false;
        let mut pri_tab = [0_i32; MENUDEPTH + 1];
        let mut enable: i32 = K_NONE;

        let modes = get_menu_cmd_modes(
            eap.cmd,
            eap.forceit != 0,
            Some(&mut noremap),
            Some(&mut unmenu),
        );
        let mut arg = eap.arg;

        loop {
            if crate::memory::cstr_prefix_bytes(arg, 8) == &b"<script>"[..8] {
                noremap = REMAP_SCRIPT;
                arg = skipwhite(arg.add(8));
                continue;
            }
            if crate::memory::cstr_prefix_bytes(arg, 8) == &b"<silent>"[..8] {
                silent = true;
                arg = skipwhite(arg.add(8));
                continue;
            }
            if crate::memory::cstr_prefix_bytes(arg, 9) == &b"<special>"[..9] {
                arg = skipwhite(arg.add(9));
                continue;
            }
            break;
        }

        if crate::memory::cstr_prefix_bytes(arg, 5) == &b"icon="[..5] {
            arg = arg.add(5);
            while !crate::memory::cstr_is_empty(arg) && crate::memory::cstr_first_byte(arg) != b' '
            {
                if crate::memory::cstr_first_byte(arg) == b'\\' {
                    crate::memory::cstr_move(arg, arg.add(1));
                }
                arg = arg.add(utfc_ptr2len(arg) as usize);
            }
            if !crate::memory::cstr_is_empty(arg) {
                *arg = NUL;
                arg = skipwhite(arg.add(1));
            }
        }

        let mut p = arg;
        while !crate::memory::cstr_is_empty(p) {
            if !ascii_isdigit(crate::memory::cstr_first_char(p))
                && crate::memory::cstr_first_byte(p) != b'.'
            {
                break;
            }
            p = p.add(1);
        }
        let mut i: usize;
        if ascii_iswhite(crate::memory::cstr_first_char(p)) {
            i = 0;
            while i < MENUDEPTH && !ascii_iswhite(crate::memory::cstr_first_char(arg)) {
                pri_tab[i] = getdigits_int(&raw mut arg, false, 0);
                if pri_tab[i] == 0 {
                    pri_tab[i] = 500;
                }
                if crate::memory::cstr_first_byte(arg) == b'.' {
                    arg = arg.add(1);
                }
                i += 1;
            }
            arg = skipwhite(arg);
        } else if eap.addr_count != 0 && eap.line2 != 0 {
            pri_tab[0] = eap.line2;
            i = 1;
        } else {
            i = 0;
        }
        while i < MENUDEPTH {
            pri_tab[i] = 500;
            i += 1;
        }
        pri_tab[MENUDEPTH] = -1;

        if (crate::memory::cstr_prefix_bytes(arg, 6) == &b"enable"[..6])
            && ascii_iswhite(crate::memory::cstr_char_at(arg, 6))
        {
            enable = K_TRUE;
            arg = skipwhite(arg.add(6));
        } else if (crate::memory::cstr_prefix_bytes(arg, 7) == &b"disable"[..7])
            && ascii_iswhite(crate::memory::cstr_char_at(arg, 7))
        {
            enable = K_FALSE;
            arg = skipwhite(arg.add(7));
        }

        if crate::memory::cstr_is_empty(arg) {
            show_menus(arg, modes);
            return;
        }

        let mut menu_path = arg;
        if crate::memory::cstr_first_byte(menu_path) == b'.' {
            semsg!(gettext(E_INVARG2), menu_path);
            return;
        }

        let mut map_to = menu_translate_tab_and_shift(arg);

        if crate::memory::cstr_is_empty(map_to) && !unmenu && enable == K_NONE {
            show_menus(menu_path, modes);
            return;
        } else if !crate::memory::cstr_is_empty(map_to) && (unmenu || enable != K_NONE) {
            semsg!(gettext(E_TRAILING_ARG), map_to);
            return;
        }

        let root_menu_ptr = get_root_menu(menu_path);

        if enable != K_NONE {
            if crate::memory::cstr_as_bytes(menu_path) == b"*" {
                menu_path = c"".as_ptr().cast_mut();
            }

            if menu_is_popup(menu_path) {
                for idx in 0..MENU_INDEX_TIP {
                    if (modes & (1 << idx)) != 0 {
                        let p = popup_mode_name(menu_path, idx as usize);
                        menu_enable_recurse(
                            &mut *root_menu_ptr,
                            p.as_ptr().cast_mut(),
                            MENU_ALL_MODES,
                            enable != 0,
                        );
                    }
                }
            }
            menu_enable_recurse(&mut *root_menu_ptr, menu_path, modes, enable != 0);
        } else if unmenu {
            if is_menus_locked() {
                return;
            }

            if crate::memory::cstr_as_bytes(menu_path) == b"*" {
                menu_path = c"".as_ptr().cast_mut();
            }

            if menu_is_popup(menu_path) {
                for idx in 0..MENU_INDEX_TIP {
                    if (modes & (1 << idx)) != 0 {
                        let p = popup_mode_name(menu_path, idx as usize);
                        remove_menu(
                            &mut *root_menu_ptr,
                            p.as_ptr().cast_mut(),
                            MENU_ALL_MODES,
                            true,
                        );
                    }
                }
            }

            remove_menu(&mut *root_menu_ptr, menu_path, modes, false);
        } else {
            if is_menus_locked() {
                return;
            }

            let map_buf_owned =
                if crate::memory::cstr_as_bytes(map_to).eq_ignore_ascii_case(b"<nop>") {
                    map_to = c"".as_ptr().cast_mut();
                    None
                } else if (modes & MENU_TIP_MODE) == 0 {
                    replace_termcodes(
                        map_to,
                        crate::memory::cstr_len(map_to),
                        false,
                        0,
                        REPTERM_DO_LT,
                        None,
                        opt_vars().p_cpo,
                    )
                } else {
                    None
                };
            if let Some(ref mb) = map_buf_owned {
                map_to = mb.as_ptr().cast_mut();
            }

            let mut menuarg = Vimmenu::new();
            menuarg.modes = modes;
            menuarg.noremap[0] = noremap;
            menuarg.silent[0] = silent;
            add_menu_path(menu_path, &mut menuarg, pri_tab.as_ptr(), map_to);

            if menu_is_popup(menu_path) {
                for idx in 0..MENU_INDEX_TIP {
                    if (modes & (1 << idx)) != 0 {
                        let p = popup_mode_name(menu_path, idx as usize);
                        menuarg.modes = modes;
                        add_menu_path(p.as_ptr(), &mut menuarg, pri_tab.as_ptr(), map_to);
                    }
                }
            }

            drop(map_buf_owned);
        }

        ui_call_update_menu();
    }
}

/// Add the menu with the given name to the menu hierarchy (`add_menu_path`).
fn add_menu_path(
    menu_path: impl crate::strings::IntoCCharPtr,
    menuarg: &mut Vimmenu,
    pri_tab: *const i32,
    call_data: impl crate::strings::IntoCCharPtr,
) -> bool {
    let menu_path = menu_path.into_c_char_ptr();
    let call_data = call_data.into_c_char_ptr();
    unsafe {
        let mut modes = menuarg.modes;
        let mut menu: *mut Vimmenu = ptr::null_mut();
        let mut pri_idx: usize = 0;
        let mut old_modes: i32 = 0;

        let mut path_name_vec: Vec<c_char> = crate::memory::cstr_to_cchar_vec(menu_path);
        let path_name = path_name_vec.as_mut_ptr();
        let root_menu_ptr = get_root_menu(menu_path);
        let mut menup: *mut Vec<Vimmenu> = root_menu_ptr;
        let mut ancestor_stack: Vec<(*mut Vec<Vimmenu>, usize)> = Vec::new();
        let mut name = path_name;

        let cleanup_erret = |ancestor_stack: &[(*mut Vec<Vimmenu>, usize)]| -> bool {
            for &(mp, idx) in ancestor_stack.iter().rev() {
                if (&(*mp))[idx].children.is_empty() {
                    (*mp).remove(idx);
                } else {
                    break;
                }
            }
            false
        };

        while !crate::memory::cstr_is_empty(name) {
            let next_name = menu_name_skip(name);
            let map_to = menutrans_lookup(name, crate::memory::cstr_len(name) as i32);
            let en_name = if !map_to.is_null() {
                let orig = name;
                name = map_to;
                orig
            } else {
                ptr::null_mut()
            };
            let dname = menu_text(name, None, None);
            if dname.as_bytes().is_empty() {
                emsg(gettext(c"E792: Empty menu name"));
                return cleanup_erret(&ancestor_stack);
            }

            let mut lower_pri_idx: usize = 0;
            let mut found_idx: usize = 0;
            menu = ptr::null_mut();
            for (idx, m) in (*menup).iter_mut().enumerate() {
                if menu_name_equal(name, m) || menu_name_equal(dname.as_ptr(), m) {
                    if crate::memory::cstr_is_empty(next_name) && !m.children.is_empty() {
                        if !g_st().sys_menu {
                            emsg(gettext(c"E330: Menu path must not lead to a sub-menu"));
                        }
                        return cleanup_erret(&ancestor_stack);
                    }
                    if !crate::memory::cstr_is_empty(next_name) && m.children.is_empty() {
                        if !g_st().sys_menu {
                            emsg(gettext(E_NOTSUBMENU));
                        }
                        return cleanup_erret(&ancestor_stack);
                    }
                    menu = m;
                    found_idx = idx;
                    break;
                }

                if (!ancestor_stack.is_empty() || menu_is_menubar(m.name.as_ptr()))
                    && m.priority <= *pri_tab.add(pri_idx)
                {
                    lower_pri_idx = idx + 1;
                }
            }

            if menu.is_null() {
                if crate::memory::cstr_is_empty(next_name) && ancestor_stack.is_empty() {
                    emsg(gettext(
                        c"E331: Must not add menu items directly to menu bar",
                    ));
                    return cleanup_erret(&ancestor_stack);
                }

                if menu_is_separator(dname.as_ptr().cast_mut())
                    && !crate::memory::cstr_is_empty(next_name)
                {
                    emsg(gettext(c"E332: Separator cannot be part of a menu path"));
                    return cleanup_erret(&ancestor_stack);
                }

                let mut new_menu = Vimmenu::new();
                new_menu.modes = modes;
                new_menu.enabled = MENU_ALL_MODES;
                new_menu.name = crate::memory::cstr_to_cstring(name);
                new_menu.dname = menu_text(
                    name,
                    Some(&mut new_menu.mnemonic),
                    Some(&mut new_menu.actext),
                );
                if !en_name.is_null() {
                    new_menu.en_name = Some(crate::memory::cstr_to_cstring(en_name));
                    new_menu.en_dname = Some(menu_text(en_name, None, None));
                }
                new_menu.priority = *pri_tab.add(pri_idx);

                (*menup).insert(lower_pri_idx, new_menu);
                menu = &raw mut (&mut (*menup))[lower_pri_idx];
                found_idx = lower_pri_idx;

                old_modes = 0;
            } else {
                old_modes = (*menu).modes;
                (*menu).modes |= modes;
                (*menu).enabled |= modes;
            }

            ancestor_stack.push((menup, found_idx));
            menup = &raw mut (*menu).children;
            name = next_name;
            if *pri_tab.add(pri_idx + 1) != -1 {
                pri_idx += 1;
            }
        }
        drop(path_name_vec);

        let amenu = (modes & (MENU_NORMAL_MODE | MENU_INSERT_MODE))
            == (MENU_NORMAL_MODE | MENU_INSERT_MODE);
        if g_st().sys_menu {
            modes &= !old_modes;
        }

        if !menu.is_null() && modes != 0 {
            let p = if call_data.is_null() {
                None
            } else {
                Some(crate::memory::cstr_to_cstring(call_data))
            };

            for i in 0..MENU_MODES {
                if (modes & (1 << i)) != 0 {
                    let mut c: c_char = 0;
                    let mut d: c_char = 0;
                    if amenu && !crate::memory::cstr_is_empty(call_data) {
                        match 1 << i {
                            MENU_VISUAL_MODE | MENU_SELECT_MODE | MENU_OP_PENDING_MODE
                            | MENU_CMDLINE_MODE => {
                                c = CTRL_C_CONST;
                            }
                            MENU_INSERT_MODE => {
                                c = CTRL_BSL_CONST;
                                d = CTRL_O;
                            }
                            _ => {}
                        }
                    }

                    if c != 0 {
                        let call_bytes = crate::memory::cstr_as_bytes(call_data);
                        let mut bytes = Vec::with_capacity(call_bytes.len() + 4);
                        bytes.push(c as u8);
                        if d != 0 {
                            bytes.push(d as u8);
                        }
                        bytes.extend_from_slice(call_bytes);
                        if c == CTRL_C_CONST {
                            bytes.push(CTRL_BSL_CONST as u8);
                            bytes.push(CTRL_G as u8);
                        }
                        (*menu).strings[i] = Some(std::ffi::CString::new(bytes).unwrap());
                    } else {
                        (*menu).strings[i] = p.clone();
                    }
                    (*menu).noremap[i] = menuarg.noremap[0];
                    (*menu).silent[i] = menuarg.silent[0];
                }
            }
        }
        true
    }
}

/// Set the (sub)menu with the given name to enabled or disabled (`menu_enable_recurse`).
fn menu_enable_recurse(
    menus: &mut [Vimmenu],
    name: impl crate::strings::IntoCCharPtr,
    modes: i32,
    enable: bool,
) -> bool {
    let name = name.into_c_char_ptr().cast_mut();
    {
        if menus.is_empty() {
            return true;
        }
        let p = menu_name_skip(name);

        let mut matched = false;
        for menu in menus.iter_mut() {
            if crate::memory::cstr_is_empty(name)
                || crate::memory::cstr_first_byte(name) == b'*'
                || menu_name_equal(name, menu)
            {
                matched = true;
                if !crate::memory::cstr_is_empty(p) {
                    if menu.children.is_empty() {
                        emsg(gettext(E_NOTSUBMENU));
                        return false;
                    }
                    if !menu_enable_recurse(&mut menu.children, p, modes, enable) {
                        return false;
                    }
                } else if enable {
                    menu.enabled |= modes;
                } else {
                    menu.enabled &= !modes;
                }

                if !crate::memory::cstr_is_empty(name)
                    && crate::memory::cstr_first_byte(name) != b'*'
                {
                    break;
                }
            }
        }
        if !crate::memory::cstr_is_empty(name)
            && crate::memory::cstr_first_byte(name) != b'*'
            && !matched
        {
            semsg!(gettext(E_NOMENU), name);
            return false;
        }

        true
    }
}

/// Remove the (sub)menu with the given name from the menu hierarchy (`remove_menu`).
fn remove_menu(
    menup: &mut Vec<Vimmenu>,
    name: impl crate::strings::IntoCCharPtr,
    modes: i32,
    silent: bool,
) -> bool {
    let name = name.into_c_char_ptr().cast_mut();
    {
        if menup.is_empty() {
            return true;
        }
        let p = menu_name_skip(name);

        let mut idx: usize = 0;
        let mut found_menu = false;
        while idx < menup.len() {
            let menu = &mut menup[idx];
            if crate::memory::cstr_is_empty(name) || menu_name_equal(name, menu) {
                if !crate::memory::cstr_is_empty(p) && menu.children.is_empty() {
                    if !silent {
                        emsg(gettext(E_NOTSUBMENU));
                    }
                    return false;
                }
                if (menu.modes & modes) != 0 {
                    if !remove_menu(&mut menu.children, p, modes, silent) {
                        return false;
                    }
                } else if !crate::memory::cstr_is_empty(name) {
                    if !silent {
                        emsg(gettext(E_MENU_ONLY_EXISTS_IN_ANOTHER_MODE));
                    }
                    return false;
                }

                if !crate::memory::cstr_is_empty(name) {
                    found_menu = true;
                    break;
                }

                menu.modes &= !modes;
                if (modes & MENU_TIP_MODE) != 0 {
                    menu.strings[MENU_INDEX_TIP as usize] = None;
                }
                if (menu.modes & MENU_ALL_MODES) == 0 {
                    menup.remove(idx);
                } else {
                    idx += 1;
                }
            } else {
                idx += 1;
            }
        }
        if !crate::memory::cstr_is_empty(name) {
            if !found_menu {
                if !silent {
                    semsg!(gettext(E_NOMENU), name);
                }
                return false;
            }

            let found = &mut menup[idx];
            found.modes &= !modes;
            for child in &found.children {
                found.modes |= child.modes;
            }
            if (modes & MENU_TIP_MODE) != 0 {
                found.strings[MENU_INDEX_TIP as usize] = None;
            }
            if (found.modes & MENU_ALL_MODES) == 0 {
                menup.remove(idx);
            }
        }

        true
    }
}

/// Export menus recursively (`menu_get_recursive`).
fn menu_get_recursive(menu: Option<&Vimmenu>, modes: i32) -> *mut TvDict {
    unsafe {
        if menu.is_none() || (menu.unwrap().modes & modes) == 0 {
            return ptr::null_mut();
        }

        let dict = tv_dict_alloc();
        tv_dict_add_str(&mut *dict, c"name", menu.unwrap().dname.as_ptr());
        tv_dict_add_nr(&mut *dict, c"priority", i64::from(menu.unwrap().priority));
        tv_dict_add_nr(
            &mut *dict,
            c"hidden",
            i64::from(menu_is_hidden(menu.unwrap().dname.as_ptr().cast_mut())),
        );

        if menu.unwrap().mnemonic != 0 {
            let mut buf = [0 as c_char; MB_MAXCHAR + 1];
            let buflen = utf_char2bytes(menu.unwrap().mnemonic, buf.as_mut_ptr());
            tv_dict_add_str_len(&mut *dict, c"shortcut", buf.as_ptr(), buflen);
        }

        if let Some(ref actext) = menu.unwrap().actext {
            tv_dict_add_str(&mut *dict, c"actext", actext.as_ptr());
        }

        if (menu.unwrap().modes & MENU_TIP_MODE) != 0
            && let Some(ref tip) = menu.unwrap().strings[MENU_INDEX_TIP as usize]
        {
            tv_dict_add_str(&mut *dict, c"tooltip", tip.as_ptr());
        }

        if menu.unwrap().children.is_empty() {
            let commands = tv_dict_alloc();
            tv_dict_add_dict(&mut *dict, c"mappings", commands);

            for (bit, mode_cstr) in MENU_MODE_CHARS.iter().enumerate() {
                if (menu.unwrap().modes & modes & (1 << bit)) != 0 {
                    let impl_dict = tv_dict_alloc();
                    let s_ptr = menu.unwrap().strings[bit]
                        .as_ref()
                        .map_or(c"".as_ptr(), |s| s.as_ptr());
                    tv_dict_add_str(
                        &mut *impl_dict,
                        c"rhs",
                        str2special_save(s_ptr, false, K_FALSE).as_ptr(),
                    );
                    tv_dict_add_nr(
                        &mut *impl_dict,
                        c"silent",
                        i64::from(menu.unwrap().silent[bit]),
                    );
                    tv_dict_add_nr(
                        &mut *impl_dict,
                        c"enabled",
                        if (menu.unwrap().enabled & (1 << bit)) != 0 {
                            1
                        } else {
                            0
                        },
                    );
                    tv_dict_add_nr(
                        &mut *impl_dict,
                        c"noremap",
                        if menu.unwrap().noremap[bit] == REMAP_NONE {
                            1
                        } else {
                            0
                        },
                    );
                    tv_dict_add_nr(
                        &mut *impl_dict,
                        c"sid",
                        if menu.unwrap().noremap[bit] == REMAP_SCRIPT {
                            1
                        } else {
                            0
                        },
                    );
                    tv_dict_add_dict(&mut *commands, mode_cstr, impl_dict);
                }
            }
        } else {
            let children_list = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
            for child in &menu.unwrap().children {
                let d = menu_get_recursive(Some(child), modes);
                if tv_dict_len(d.as_ref()) > 0 {
                    tv_list_append_dict(&mut *children_list, d);
                }
            }
            tv_dict_add_list(&mut *dict, c"submenus", children_list);
        }
        dict
    }
}

/// Export menus matching path `path_name` (`menu_get`).
pub fn menu_get(
    path_name: impl crate::strings::IntoCCharPtr,
    modes: i32,
    list: &mut TvList,
) -> bool {
    let path_name = path_name.into_c_char_ptr();
    unsafe {
        let root = get_root_menu(path_name);
        if !crate::memory::cstr_is_empty(path_name) {
            let menu = find_menu(&mut *root, path_name, modes);
            if menu.is_null() {
                return false;
            }
            let d = menu_get_recursive(menu.as_ref(), modes);
            if !d.is_null() && tv_dict_len(d.as_ref()) > 0 {
                tv_list_append_dict(list, d);
            }
            return true;
        }
        for menu in &*root {
            let d = menu_get_recursive(Some(menu), modes);
            if !d.is_null() && tv_dict_len(d.as_ref()) > 0 {
                tv_list_append_dict(list, d);
            }
        }
        true
    }
}

/// Find menu matching `path_name` and `modes` (`find_menu`).
fn find_menu(
    mut menus: &mut [Vimmenu],
    path_name: impl crate::strings::IntoCCharPtr,
    modes: i32,
) -> *mut Vimmenu {
    let path_name = path_name.into_c_char_ptr();
    unsafe {
        debug_assert!(!crate::memory::cstr_is_empty(path_name));
        let mut saved_name_vec: Vec<c_char> = crate::memory::cstr_to_cchar_vec(path_name);
        let mut name = saved_name_vec.as_mut_ptr();
        let mut found: *mut Vimmenu = ptr::null_mut();

        while !crate::memory::cstr_is_empty(name) {
            let p = menu_name_skip(name);
            found = ptr::null_mut();
            for menu in menus.iter_mut() {
                if menu_name_equal(name, menu) {
                    if !crate::memory::cstr_is_empty(p) && menu.children.is_empty() {
                        emsg(gettext(E_NOTSUBMENU));
                        return ptr::null_mut();
                    } else if (menu.modes & modes) == 0 {
                        emsg(gettext(E_MENU_ONLY_EXISTS_IN_ANOTHER_MODE));
                        return ptr::null_mut();
                    } else if crate::memory::cstr_is_empty(p) {
                        return menu;
                    }
                    found = menu;
                    break;
                }
            }

            if found.is_null() {
                semsg!(gettext(E_NOMENU), name);
                break;
            }
            name = p;
            debug_assert!(!crate::memory::cstr_is_empty(name));
            menus = &mut (*found).children;
        }

        found
    }
}

/// Show the mapping associated with a menu item or hierarchy in a sub-menu (`show_menus`).
fn show_menus(path_name: impl crate::strings::IntoCCharPtr, modes: i32) -> bool {
    let path_name = path_name.into_c_char_ptr();
    unsafe {
        let mut menu: *mut Vimmenu = ptr::null_mut();
        if !crate::memory::cstr_is_empty(path_name) {
            menu = find_menu(&mut *get_root_menu(path_name), path_name, modes);
            if menu.is_null() {
                return false;
            }
        }

        menu_st().menus_locked += 1;
        msg_puts_title(gettext(c"\n--- Menus ---"));
        show_menus_recursive(menu.as_mut(), modes, 0);
        menu_st().menus_locked -= 1;
        true
    }
}

/// Recursively show the mappings associated with the menus under the given one (`show_menus_recursive`).
fn show_menus_recursive(mut menu: Option<&mut Vimmenu>, modes: i32, mut depth: i32) {
    {
        if menu.is_some() && (menu.as_deref_mut().unwrap().modes & modes) == 0 {
            return;
        }

        if menu.is_some() {
            msg_putchar(b'\n' as i32);
            if g_st().got_int {
                return;
            }
            for _ in 0..depth {
                msg_puts(c"  ");
            }
            if menu.as_deref_mut().unwrap().priority != 0 {
                msg_outnum(menu.as_deref_mut().unwrap().priority);
                msg_puts(c" ");
            }
            msg_outtrans(menu.as_deref_mut().unwrap().name.as_ptr(), HLF_D, false);
        }

        if menu.is_some() && menu.as_deref_mut().unwrap().children.is_empty() {
            for (bit, mode_cstr) in MENU_MODE_CHARS.iter().enumerate() {
                if (menu.as_deref_mut().unwrap().modes & modes & (1 << bit)) != 0 {
                    msg_putchar(b'\n' as i32);
                    if g_st().got_int {
                        return;
                    }
                    for _ in 0..(depth + 2) {
                        msg_puts(c"  ");
                    }
                    msg_puts(mode_cstr.as_ptr());
                    if menu.as_deref_mut().unwrap().noremap[bit] == REMAP_NONE {
                        msg_putchar(b'*' as i32);
                    } else if menu.as_deref_mut().unwrap().noremap[bit] == REMAP_SCRIPT {
                        msg_putchar(b'&' as i32);
                    } else {
                        msg_putchar(b' ' as i32);
                    }
                    if menu.as_deref_mut().unwrap().silent[bit] {
                        msg_putchar(b's' as i32);
                    } else {
                        msg_putchar(b' ' as i32);
                    }
                    if (menu.as_deref_mut().unwrap().modes
                        & menu.as_deref_mut().unwrap().enabled
                        & (1 << bit))
                        == 0
                    {
                        msg_putchar(b'-' as i32);
                    } else {
                        msg_putchar(b' ' as i32);
                    }
                    msg_puts(c" ");
                    let s_bytes = menu.as_deref_mut().unwrap().strings[bit]
                        .as_ref()
                        .map_or(&[][..], |s| s.as_bytes());
                    if s_bytes.is_empty() {
                        msg_puts_hl(c"<Nop>", HLF_8, false);
                    } else {
                        msg_outtrans_special(
                            menu.as_deref_mut().unwrap().strings[bit]
                                .as_ref()
                                .unwrap()
                                .as_ptr(),
                            false,
                            0,
                        );
                    }
                }
            }
        } else {
            let children = if let Some(menu) = menu {
                menu.children.as_mut_slice()
            } else {
                depth -= 1;
                g_st().root_menu.as_mut_slice()
            };

            for child in children {
                if g_st().got_int {
                    break;
                }
                if !menu_is_hidden(child.dname.as_ptr().cast_mut()) {
                    show_menus_recursive(Some(&mut *child), modes, depth + 1);
                }
            }
        }
    }
}

/// Work out what to complete when doing command line completion of menu names (`set_context_in_menu_cmd`).
pub fn set_context_in_menu_cmd(
    xp: &mut ExpandCtx,
    cmd: impl crate::strings::IntoCCharPtr,
    mut arg: *mut c_char,
    forceit: bool,
) -> *mut c_char {
    let cmd = cmd.into_c_char_ptr();
    unsafe {
        let mut path_name: *mut c_char = ptr::null_mut();
        let mut unmenu = false;

        xp.xp_context = EXPAND_UNSUCCESSFUL;

        let mut p = arg;
        while !crate::memory::cstr_is_empty(p) {
            if !ascii_isdigit(crate::memory::cstr_first_char(p))
                && crate::memory::cstr_first_byte(p) != b'.'
            {
                break;
            }
            p = p.add(1);
        }

        if !ascii_iswhite(crate::memory::cstr_first_char(p)) {
            if (crate::memory::cstr_prefix_bytes(arg, 6) == &b"enable"[..6])
                && (crate::memory::cstr_byte_at(arg, 6) == 0
                    || ascii_iswhite(crate::memory::cstr_char_at(arg, 6)))
            {
                p = arg.add(6);
            } else if (crate::memory::cstr_prefix_bytes(arg, 7) == &b"disable"[..7])
                && (crate::memory::cstr_byte_at(arg, 7) == 0
                    || ascii_iswhite(crate::memory::cstr_char_at(arg, 7)))
            {
                p = arg.add(7);
            } else {
                p = arg;
            }
        }

        while !crate::memory::cstr_is_empty(p) && ascii_iswhite(crate::memory::cstr_first_char(p)) {
            p = p.add(1);
        }

        let mut after_dot = p;
        arg = p;

        while !crate::memory::cstr_is_empty(p) && !ascii_iswhite(crate::memory::cstr_first_char(p))
        {
            if (crate::memory::cstr_first_byte(p) == b'\\' || *p == CTRL_V_CONST)
                && crate::memory::cstr_byte_at(p, 1) != 0
            {
                p = p.add(1);
            } else if crate::memory::cstr_first_byte(p) == b'.' {
                after_dot = p.add(1);
            }
            p = p.add(1);
        }

        let expand_menus = !((crate::memory::cstr_first_byte(cmd) == b't'
            && crate::memory::cstr_byte_at(cmd, 1) == b'e')
            || crate::memory::cstr_first_byte(cmd) == b'p');
        menu_st().expand_emenu = crate::memory::cstr_first_byte(cmd) == b'e';
        if expand_menus && ascii_iswhite(crate::memory::cstr_first_char(p)) {
            return ptr::null_mut();
        }
        if crate::memory::cstr_is_empty(p) {
            menu_st().expand_modes = get_menu_cmd_modes(cmd, forceit, None, Some(&mut unmenu));
            if !unmenu {
                menu_st().expand_modes = MENU_ALL_MODES;
            }

            let mut menu_list: *const Vec<Vimmenu> = &raw const g_st().root_menu;
            let mut path_name_vec: Vec<c_char>;
            if after_dot > arg {
                let path_len = after_dot.offset_from(arg) as usize;
                path_name_vec = vec![0; path_len];
                path_name = path_name_vec.as_mut_ptr();
                xstrlcpy(path_name, arg, path_len);
            }
            let mut name = path_name;
            while !crate::memory::cstr_is_empty(name) {
                p = menu_name_skip(name);
                let mut found: *const Vimmenu = ptr::null();
                for menu in &*menu_list {
                    if menu_name_equal(name, menu) {
                        if (!crate::memory::cstr_is_empty(p) && menu.children.is_empty())
                            || (menu.modes & menu_st().expand_modes) == 0
                        {
                            return ptr::null_mut();
                        }
                        found = menu;
                        break;
                    }
                }
                if found.is_null() {
                    return ptr::null_mut();
                }
                name = p;
                menu_list = &raw const (*found).children;
            }

            xp.xp_context = if expand_menus {
                EXPAND_MENUNAMES
            } else {
                EXPAND_MENUS
            };
            xp.xp_pattern = after_dot;
            menu_st().expand_menu = menu_list;
        } else {
            xp.xp_context = EXPAND_NOTHING;
        }
        ptr::null_mut()
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of (sub)menus (not entries) (`get_menu_name`).
pub fn get_menu_name(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    unsafe {
        if idx == 0 {
            menu_st().get_menu_name_idx = 0;
            menu_st().get_menu_name_advance = false;
        }

        if menu_st().expand_menu.is_null() {
            return ptr::null_mut();
        }
        let menus = (*menu_st().expand_menu).as_slice();

        while menu_st().get_menu_name_idx < menus.len() {
            let menu = &menus[menu_st().get_menu_name_idx];
            let dname_ptr = menu.dname.as_ptr().cast_mut();
            if menu_is_hidden(dname_ptr) || menu_is_separator(dname_ptr) || menu.children.is_empty()
            {
                menu_st().get_menu_name_idx += 1;
            } else {
                break;
            }
        }

        if menu_st().get_menu_name_idx >= menus.len() {
            return ptr::null_mut();
        }
        let menu = &menus[menu_st().get_menu_name_idx];

        let str_ptr = if (menu.modes & menu_st().expand_modes) != 0 {
            if menu_st().get_menu_name_advance {
                menu.en_dname
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            } else {
                if menu.en_dname.is_none() {
                    menu_st().get_menu_name_advance = true;
                }
                menu.dname.as_ptr().cast_mut()
            }
        } else {
            c"".as_ptr().cast_mut()
        };

        if menu_st().get_menu_name_advance {
            menu_st().get_menu_name_idx += 1;
        }

        menu_st().get_menu_name_advance = !menu_st().get_menu_name_advance;

        str_ptr
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of menus and menu entries (`get_menu_names`).
pub fn get_menu_names(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    unsafe {
        if idx == 0 {
            menu_st().get_menu_names_idx = 0;
            menu_st().get_menu_names_advance = false;
        }

        if menu_st().expand_menu.is_null() {
            return ptr::null_mut();
        }
        let menus = (*menu_st().expand_menu).as_slice();

        while menu_st().get_menu_names_idx < menus.len() && {
            let dname = menus[menu_st().get_menu_names_idx]
                .dname
                .as_ptr()
                .cast_mut();
            let dlen = menus[menu_st().get_menu_names_idx].dname.as_bytes().len();
            menu_is_hidden(dname)
                || (menu_st().expand_emenu && menu_is_separator(dname))
                || (dlen > 0 && crate::memory::cstr_byte_at(dname, dlen - 1) == b'.')
        } {
            menu_st().get_menu_names_idx += 1;
        }

        if menu_st().get_menu_names_idx >= menus.len() {
            return ptr::null_mut();
        }
        let menu = &menus[menu_st().get_menu_names_idx];

        let str_ptr = if (menu.modes & menu_st().expand_modes) != 0 {
            if !menu.children.is_empty() {
                if menu_st().get_menu_names_advance {
                    xstrlcpy(
                        &raw mut menu_st().get_menu_names_tbuffer[0],
                        menu.en_dname.as_ref().map_or(c"".as_ptr(), |s| s.as_ptr()),
                        TBUFFER_LEN,
                    );
                } else {
                    xstrlcpy(
                        &raw mut menu_st().get_menu_names_tbuffer[0],
                        menu.dname.as_ptr(),
                        TBUFFER_LEN,
                    );
                    if menu.en_dname.is_none() {
                        menu_st().get_menu_names_advance = true;
                    }
                }
                crate::memory::cstr_cat(&raw mut menu_st().get_menu_names_tbuffer[0], c"\x01");
                &raw mut menu_st().get_menu_names_tbuffer[0]
            } else if menu_st().get_menu_names_advance {
                menu.en_dname
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            } else {
                if menu.en_dname.is_none() {
                    menu_st().get_menu_names_advance = true;
                }
                menu.dname.as_ptr().cast_mut()
            }
        } else {
            c"".as_ptr().cast_mut()
        };

        if menu_st().get_menu_names_advance {
            menu_st().get_menu_names_idx += 1;
        }

        menu_st().get_menu_names_advance = !menu_st().get_menu_names_advance;

        str_ptr
    }
}

/// Skip over this element of the menu path and return the start of the next element (`menu_name_skip`).
fn menu_name_skip(name: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let name = name.into_c_char_ptr().cast_mut();
    unsafe {
        let mut p = name;
        while !crate::memory::cstr_is_empty(p) && crate::memory::cstr_first_byte(p) != b'.' {
            if crate::memory::cstr_first_byte(p) == b'\\' || *p == CTRL_V_CONST {
                crate::memory::cstr_move(p, p.add(1));
                if crate::memory::cstr_is_empty(p) {
                    break;
                }
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        if !crate::memory::cstr_is_empty(p) {
            *p = NUL;
            p = p.add(1);
        }
        p
    }
}

/// Return true when `name` matches with menu `menu` (`menu_name_equal`).
fn menu_name_equal(name: impl crate::strings::IntoCCharPtr, menu: &Vimmenu) -> bool {
    let name = name.into_c_char_ptr();
    if let Some(ref en_name) = menu.en_name
        && (menu_namecmp(name, en_name.as_ptr())
            || menu
                .en_dname
                .as_ref()
                .is_some_and(|ed| menu_namecmp(name, ed.as_ptr())))
    {
        return true;
    }
    menu_namecmp(name, menu.name.as_ptr()) || menu_namecmp(name, menu.dname.as_ptr())
}

fn menu_namecmp(
    name: impl crate::strings::IntoCCharPtr,
    mname: impl crate::strings::IntoCCharPtr,
) -> bool {
    let name = name.into_c_char_ptr();
    let mname = mname.into_c_char_ptr();
    unsafe {
        let mut i: usize = 0;
        while crate::memory::cstr_byte_at(name, i) != 0 && *name.add(i) != TAB {
            if *name.add(i) != *mname.add(i) {
                break;
            }
            i += 1;
        }
        (crate::memory::cstr_byte_at(name, i) == 0 || *name.add(i) == TAB)
            && (crate::memory::cstr_byte_at(mname, i) == 0 || *mname.add(i) == TAB)
    }
}

/// Return the `MENU_MODES` specified by menu command `cmd` (`get_menu_cmd_modes`).
pub fn get_menu_cmd_modes(
    cmd: impl crate::strings::IntoCCharPtr,
    forceit: bool,
    noremap: Option<&mut i32>,
    unmenu: Option<&mut bool>,
) -> i32 {
    let mut cmd = cmd.into_c_char_ptr();
    unsafe {
        let first = *cmd;
        cmd = cmd.add(1);
        let modes = match first as u8 {
            b'v' => MENU_VISUAL_MODE | MENU_SELECT_MODE,
            b'x' => MENU_VISUAL_MODE,
            b's' => MENU_SELECT_MODE,
            b'o' => MENU_OP_PENDING_MODE,
            b'i' => MENU_INSERT_MODE,
            b't' => {
                if crate::memory::cstr_first_byte(cmd) == b'l' {
                    cmd = cmd.add(1);
                    MENU_TERMINAL_MODE
                } else {
                    MENU_TIP_MODE
                }
            }
            b'c' => MENU_CMDLINE_MODE,
            b'a' => {
                MENU_INSERT_MODE
                    | MENU_CMDLINE_MODE
                    | MENU_NORMAL_MODE
                    | MENU_VISUAL_MODE
                    | MENU_SELECT_MODE
                    | MENU_OP_PENDING_MODE
            }
            b'n' if crate::memory::cstr_first_byte(cmd) != b'o' => MENU_NORMAL_MODE,
            _ => {
                cmd = cmd.sub(1);
                if forceit {
                    MENU_INSERT_MODE | MENU_CMDLINE_MODE
                } else {
                    MENU_NORMAL_MODE | MENU_VISUAL_MODE | MENU_SELECT_MODE | MENU_OP_PENDING_MODE
                }
            }
        };

        if let Some(noremap) = noremap {
            *noremap = if crate::memory::cstr_first_byte(cmd) == b'n' {
                REMAP_NONE
            } else {
                REMAP_YES
            };
        }
        if let Some(unmenu) = unmenu {
            *unmenu = crate::memory::cstr_first_byte(cmd) == b'u';
        }
        modes
    }
}

/// Return the string representation of the menu modes (`get_menu_mode_str`).
const fn get_menu_mode_str(modes: i32) -> &'static std::ffi::CStr {
    const ALL_A: i32 = MENU_INSERT_MODE
        | MENU_CMDLINE_MODE
        | MENU_NORMAL_MODE
        | MENU_VISUAL_MODE
        | MENU_SELECT_MODE
        | MENU_OP_PENDING_MODE;
    const ALL_DEF: i32 =
        MENU_NORMAL_MODE | MENU_VISUAL_MODE | MENU_SELECT_MODE | MENU_OP_PENDING_MODE;
    const ALL_BANG: i32 = MENU_INSERT_MODE | MENU_CMDLINE_MODE;
    const ALL_V: i32 = MENU_VISUAL_MODE | MENU_SELECT_MODE;

    if (modes & ALL_A) == ALL_A {
        c"a"
    } else if (modes & ALL_DEF) == ALL_DEF {
        c" "
    } else if (modes & ALL_BANG) == ALL_BANG {
        c"!"
    } else if (modes & ALL_V) == ALL_V {
        c"v"
    } else if (modes & MENU_VISUAL_MODE) != 0 {
        c"x"
    } else if (modes & MENU_SELECT_MODE) != 0 {
        c"s"
    } else if (modes & MENU_OP_PENDING_MODE) != 0 {
        c"o"
    } else if (modes & MENU_INSERT_MODE) != 0 {
        c"i"
    } else if (modes & MENU_TERMINAL_MODE) != 0 {
        c"tl"
    } else if (modes & MENU_CMDLINE_MODE) != 0 {
        c"c"
    } else if (modes & MENU_NORMAL_MODE) != 0 {
        c"n"
    } else if (modes & MENU_TIP_MODE) != 0 {
        c"t"
    } else {
        c""
    }
}

/// Modify a menu name starting with `"PopUp"` to include the mode character (`popup_mode_name`).
fn popup_mode_name(name: impl crate::strings::IntoCCharPtr, idx: usize) -> std::ffi::CString {
    let name = name.into_c_char_ptr();
    {
        let name_bytes = crate::memory::cstr_as_bytes(name);
        debug_assert!(name_bytes.len() >= 4);

        let mode_chars = MENU_MODE_CHARS[idx].to_bytes();
        let mut out = Vec::with_capacity(name_bytes.len() + mode_chars.len());
        let split_at = name_bytes.len().min(5);
        out.extend_from_slice(&name_bytes[..split_at]);
        out.extend_from_slice(mode_chars);
        out.extend_from_slice(&name_bytes[split_at..]);
        std::ffi::CString::new(out).unwrap_or_default()
    }
}

/// Duplicate the menu item text and then process mnemonic key and accelerator text (`menu_text`).
fn menu_text(
    str: impl crate::strings::IntoCCharPtr,
    mut mnemonic: Option<&mut i32>,
    actext: Option<&mut Option<std::ffi::CString>>,
) -> std::ffi::CString {
    let str = str.into_c_char_ptr();
    unsafe {
        let mut p = vim_strchr(str, i32::from(TAB as u8));
        let mut text_vec: Vec<c_char> = if !p.is_null() {
            if let Some(actext_out) = actext {
                *actext_out = Some(crate::memory::cstr_to_cstring(p.add(1)));
            }
            let len = p.offset_from(str) as usize;
            let mut v = Vec::with_capacity(len + 1);
            v.extend_from_slice(std::slice::from_raw_parts(str, len));
            v.push(0);
            v
        } else {
            crate::memory::cstr_to_cchar_vec(str)
        };
        let text = text_vec.as_mut_ptr();

        p = text;
        while !p.is_null() {
            p = vim_strchr(p, b'&' as i32);
            if !p.is_null() {
                if crate::memory::cstr_byte_at(p, 1) == 0 {
                    break;
                }
                if crate::memory::cstr_byte_at(p, 1) != b'&'
                    && let Some(ref mut mnemonic) = mnemonic
                {
                    **mnemonic = i32::from(crate::memory::cstr_byte_at(p, 1));
                }
                crate::memory::cstr_move(p, p.add(1));
                p = p.add(1);
            }
        }
        crate::memory::cstr_to_cstring(text)
    }
}

/// Return true if `name` can be a menu in the MenuBar (`menu_is_menubar`).
pub fn menu_is_menubar(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    unsafe {
        !menu_is_popup(name)
            && !menu_is_toolbar(name)
            && !menu_is_winbar(name)
            && *name != MNU_HIDDEN_CHAR
    }
}

/// Return true if `name` is a popup menu name (`menu_is_popup`).
pub fn menu_is_popup(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    crate::memory::cstr_prefix_bytes(name, 5) == &b"PopUp"[..5]
}

/// Return true if `name` is a toolbar menu name (`menu_is_toolbar`).
pub fn menu_is_toolbar(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    crate::memory::cstr_prefix_bytes(name, 7) == &b"ToolBar"[..7]
}

/// Return true if the name is a menu separator identifier (`menu_is_separator`).
pub fn menu_is_separator(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    {
        let len = crate::memory::cstr_len(name);
        len > 0
            && crate::memory::cstr_first_byte(name) == b'-'
            && crate::memory::cstr_byte_at(name, len - 1) == b'-'
    }
}

/// True if a popup menu or starts with `MNU_HIDDEN_CHAR` (`menu_is_hidden`).
fn menu_is_hidden(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    unsafe {
        *name == MNU_HIDDEN_CHAR
            || (menu_is_popup(name) && crate::memory::cstr_byte_at(name, 5) != 0)
    }
}

fn get_menu_mode() -> i32 {
    if (g_st().state_global & MODE_TERMINAL) != 0 {
        return MENU_INDEX_TERMINAL;
    }
    if g_st().visual_global.active {
        if g_st().visual_global.select {
            return MENU_INDEX_SELECT;
        }
        return MENU_INDEX_VISUAL;
    }
    if (g_st().state_global & MODE_INSERT) != 0 {
        return MENU_INDEX_INSERT;
    }
    if (g_st().state_global & MODE_CMDLINE) != 0
        || g_st().state_global == MODE_ASKMORE
        || g_st().state_global == MODE_HITRETURN
    {
        return MENU_INDEX_CMDLINE;
    }
    if g_st().finish_op {
        return MENU_INDEX_OP_PENDING;
    }
    if (g_st().state_global & MODE_NORMAL) != 0 {
        return MENU_INDEX_NORMAL;
    }
    if (g_st().state_global & MODE_LANGMAP) != 0 {
        return MENU_INDEX_INSERT;
    }
    MENU_INDEX_INVALID
}

pub fn get_menu_mode_flag() -> i32 {
    let mode = get_menu_mode();
    if mode == MENU_INDEX_INVALID {
        0
    } else {
        1 << mode
    }
}

/// Display the Special `"PopUp"` menu as a pop-up at the current mouse position (`show_popupmenu`).
pub fn show_popupmenu() {
    unsafe {
        let menu_mode = get_menu_mode();
        if menu_mode == MENU_INDEX_INVALID {
            return;
        }
        let mode_cstr = MENU_MODE_CHARS[menu_mode as usize];
        let mode = mode_cstr.as_ptr().cast_mut();
        let mode_len = mode_cstr.to_bytes().len();

        apply_autocmds(EVENT_MENUPOPUP, mode, ptr::null_mut(), false, g_st().curbuf);

        let mut menu: *mut Vimmenu = ptr::null_mut();
        for m in &mut g_st().root_menu {
            if (b"PopUp" == crate::memory::cstr_prefix_bytes(m.name.as_ptr(), 5))
                && ((&crate::memory::cstr_slice(m.name.as_ptr(), (5) + (mode_len))[5..])
                    == crate::memory::cstr_prefix_bytes(mode, mode_len))
            {
                menu = m;
                break;
            }
        }

        if menu.is_null() || (*menu).children.is_empty() {
            return;
        }

        pum_show_popupmenu(&mut *menu);
    }
}

/// Execute `menu` (`execute_menu`).
pub fn execute_menu(eap: &Exarg, menu: &mut Vimmenu, mode_idx: i32) {
    {
        let mut idx = mode_idx;

        if idx < 0 {
            if (g_st().state_global & MODE_TERMINAL) != 0 {
                idx = MENU_INDEX_TERMINAL;
            } else if (g_st().state_global & MODE_CMDLINE) != 0 {
                idx = MENU_INDEX_CMDLINE;
            } else if (get_real_state() & MODE_VISUAL) != 0 {
                idx = MENU_INDEX_VISUAL;
            } else if ((g_st().state_global & MODE_INSERT) != 0 || g_st().restart_edit != 0)
                && g_st().current_sctx.sc_sid == 0
            {
                idx = MENU_INDEX_INSERT;
            } else if eap.addr_count != 0 {
                idx = MENU_INDEX_VISUAL;

                let tpos: CursorPos;
                if curbuf_mut().b_visual.vi_start.lnum == eap.line1
                    && curbuf_mut().b_visual.vi_end.lnum == eap.line2
                {
                    g_st().visual_global.mode = curbuf_mut().b_visual.vi_mode;
                    tpos = curbuf_mut().b_visual.vi_end;
                    curwin_mut().w_cursor = curbuf_mut().b_visual.vi_start;
                    curwin_mut().w_curswant = curbuf_mut().b_visual.vi_curswant;
                } else {
                    g_st().visual_global.mode = b'V' as i32;
                    curwin_mut().w_cursor.lnum = eap.line1;
                    curwin_mut().w_cursor.col = 1;
                    tpos = CursorPos {
                        lnum: eap.line2,
                        col: MAXCOL,
                        coladd: 0,
                    };
                }

                g_st().visual_global.active = true;
                g_st().visual_global.reselect = 1;
                check_cursor(curwin_mut());
                g_st().visual_global.start = curwin_mut().w_cursor;
                curwin_mut().w_cursor = tpos;

                check_cursor(curwin_mut());

                if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
                    && gchar_cursor() != i32::from(NUL)
                {
                    curwin_mut().w_cursor.col += 1;
                }
            }
        }

        if idx == MENU_INDEX_INVALID {
            idx = MENU_INDEX_NORMAL;
        }

        let u_idx = idx as usize;
        if let Some(ref cmd_str) = menu.strings[u_idx]
            && (menu.modes & (1 << idx)) != 0
        {
            let cmd_ptr = cmd_str.as_ptr().cast_mut();
            if g_st().current_sctx.sc_sid != 0 {
                let mut save_state = SaveState::new();
                g_st().ex_normal_busy += 1;
                save_current_state(&mut save_state);
                exec_normal_cmd(cmd_ptr, menu.noremap[u_idx], menu.silent[u_idx]);
                restore_current_state(&mut save_state);
                g_st().ex_normal_busy -= 1;
            } else {
                ins_typebuf(cmd_ptr, menu.noremap[u_idx], 0, true, menu.silent[u_idx]);
            }
        } else {
            let mode = match idx {
                MENU_INDEX_VISUAL => c"Visual",
                MENU_INDEX_SELECT => c"Select",
                MENU_INDEX_OP_PENDING => c"Op-pending",
                MENU_INDEX_TERMINAL => c"Terminal",
                MENU_INDEX_INSERT => c"Insert",
                MENU_INDEX_CMDLINE => c"Cmdline",
                _ => c"Normal",
            };
            semsg!(
                gettext(c"E335: Menu not defined for %s mode"),
                mode.as_ptr(),
            );
        }
    }
}

/// Lookup a menu by the descriptor name e.g. `"File.New"` (`menu_getbyname`).
fn menu_getbyname(name_arg: impl crate::strings::IntoCCharPtr) -> *mut Vimmenu {
    let name_arg = name_arg.into_c_char_ptr();
    unsafe {
        let mut saved_name_vec: Vec<c_char> = crate::memory::cstr_to_cchar_vec(name_arg);
        let saved_name = saved_name_vec.as_mut_ptr();
        let mut menus = (&mut *get_root_menu(saved_name)).as_mut_slice();
        let mut name = saved_name;
        let mut gave_emsg = false;
        let mut found: *mut Vimmenu = ptr::null_mut();
        while !crate::memory::cstr_is_empty(name) {
            let p = menu_name_skip(name);
            found = ptr::null_mut();

            for menu in menus.iter_mut() {
                if menu_name_equal(name, menu) {
                    if crate::memory::cstr_is_empty(p) && !menu.children.is_empty() {
                        emsg(gettext(c"E333: Menu path must lead to a menu item"));
                        gave_emsg = true;
                    } else if !crate::memory::cstr_is_empty(p) && menu.children.is_empty() {
                        emsg(gettext(E_NOTSUBMENU));
                    } else {
                        found = menu;
                    }
                    break;
                }
            }
            if found.is_null() || crate::memory::cstr_is_empty(p) {
                break;
            }
            menus = (*found).children.as_mut_slice();
            name = p;
        }
        drop(saved_name_vec);
        if found.is_null() {
            if !gave_emsg {
                semsg!(gettext(c"E334: Menu not found: %s"), name_arg);
            }
            return ptr::null_mut();
        }

        found
    }
}

/// `:emenu` (`ex_emenu`).
pub fn ex_emenu(eap: &mut Exarg) {
    unsafe {
        let mut arg = eap.arg;
        let mut mode_idx = MENU_INDEX_INVALID;

        if !crate::memory::cstr_is_empty(arg) && ascii_iswhite(crate::memory::cstr_char_at(arg, 1))
        {
            match crate::memory::cstr_first_byte(arg) {
                b'n' => mode_idx = MENU_INDEX_NORMAL,
                b'v' => mode_idx = MENU_INDEX_VISUAL,
                b's' => mode_idx = MENU_INDEX_SELECT,
                b'o' => mode_idx = MENU_INDEX_OP_PENDING,
                b't' => mode_idx = MENU_INDEX_TERMINAL,
                b'i' => mode_idx = MENU_INDEX_INSERT,
                b'c' => mode_idx = MENU_INDEX_CMDLINE,
                _ => {
                    semsg!(gettext(E_INVARG2), arg);
                    return;
                }
            }
            arg = skipwhite(arg.add(2));
        }

        let menu = menu_getbyname(arg);
        if menu.is_null() {
            return;
        }

        execute_menu(eap, &mut *menu, mode_idx);
    }
}

/// Given a menu descriptor, e.g. `"File.New"`, find it in the menu hierarchy (`menu_find`).
pub fn menu_find(path_name: impl crate::strings::IntoCCharPtr) -> *mut Vimmenu {
    let path_name = path_name.into_c_char_ptr();
    unsafe {
        let mut menus = (&mut *get_root_menu(path_name)).as_mut_slice();
        let mut saved_name_vec: Vec<c_char> = crate::memory::cstr_to_cchar_vec(path_name);
        let mut name = saved_name_vec.as_mut_ptr();
        let mut found: *mut Vimmenu = ptr::null_mut();
        while !crate::memory::cstr_is_empty(name) {
            let p = menu_name_skip(name);
            found = ptr::null_mut();

            for menu in menus.iter_mut() {
                if menu_name_equal(name, menu) {
                    if menu.children.is_empty() {
                        if crate::memory::cstr_is_empty(p) {
                            emsg(gettext(c"E336: Menu path must lead to a sub-menu"));
                        } else {
                            emsg(gettext(E_NOTSUBMENU));
                        }
                        return ptr::null_mut();
                    }
                    if crate::memory::cstr_is_empty(p) {
                        return menu;
                    }
                    found = menu;
                    break;
                }
            }
            if found.is_null() {
                break;
            }

            menus = (*found).children.as_mut_slice();
            name = p;
        }

        if found.is_null() {
            emsg(gettext(c"E337: Menu not found - check menu names"));
        }
        found
    }
}

/// `:menutrans` (`ex_menutranslate`).
pub fn ex_menutranslate(eap: &mut Exarg) {
    unsafe {
        let mut arg = eap.arg;
        let menutrans = &mut menu_st().menutrans_ga;

        if (crate::memory::cstr_prefix_bytes(arg, 5) == &b"clear"[..5])
            && ends_excmd(i32::from(*skipwhite(arg.add(5)) as u8)) != 0
        {
            menutrans.clear();
            del_menutrans_vars();
        } else {
            let from_arg = arg;
            arg = menu_skip_part(arg);
            let to_arg = skipwhite(arg);
            *arg = NUL;
            arg = menu_skip_part(to_arg);
            if arg == to_arg {
                emsg(gettext(E_INVARG));
            } else {
                let mut from_vec: Vec<c_char> = crate::memory::cstr_to_cchar_vec(from_arg);
                let from = from_vec.as_mut_ptr();
                let from_noamp_cstr = menu_text(from, None, None);
                let to_len = arg.offset_from(to_arg) as usize;
                let mut to_vec: Vec<c_char> = Vec::with_capacity(to_len + 1);
                to_vec.extend_from_slice(std::slice::from_raw_parts(to_arg, to_len));
                to_vec.push(0);
                let to = to_vec.as_mut_ptr();
                menu_translate_tab_and_shift(from);
                menu_translate_tab_and_shift(to);
                menu_unescape_name(from);
                menu_unescape_name(to);
                let from_cstr = crate::memory::cstr_to_cstring(from);
                let to_cstr = crate::memory::cstr_to_cstring(to);
                menutrans.push(Menutrans {
                    from: from_cstr,
                    from_noamp: from_noamp_cstr,
                    to: to_cstr,
                });
            }
        }
    }
}

/// Find the character just after one part of a menu name (`menu_skip_part`).
fn menu_skip_part(p: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let mut p = p.into_c_char_ptr().cast_mut();
    unsafe {
        while !crate::memory::cstr_is_empty(p)
            && crate::memory::cstr_first_byte(p) != b'.'
            && !ascii_iswhite(crate::memory::cstr_first_char(p))
        {
            if (crate::memory::cstr_first_byte(p) == b'\\' || *p == CTRL_V_CONST)
                && crate::memory::cstr_byte_at(p, 1) != 0
            {
                p = p.add(1);
            }
            p = p.add(1);
        }
        p
    }
}

/// Lookup part of a menu name in the translations (`menutrans_lookup`).
fn menutrans_lookup(name: *mut c_char, len: i32) -> *mut c_char {
    unsafe {
        let menutrans = &menu_st().menutrans_ga;
        let u_len = len as usize;

        for entry in menutrans {
            let from_ptr = entry.from.as_ptr();
            if crate::memory::cstr_prefix_bytes(name, u_len)
                .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(from_ptr, u_len))
                && crate::memory::cstr_byte_at(from_ptr, u_len) == 0
            {
                return entry.to.as_ptr().cast_mut();
            }
        }

        let c = *name.add(u_len);
        *name.add(u_len) = NUL;
        let dname = menu_text(name, None, None);
        *name.add(u_len) = c;
        for entry in menutrans {
            if crate::memory::cstr_as_bytes(dname.as_ptr())
                .eq_ignore_ascii_case(crate::memory::cstr_as_bytes(entry.from_noamp.as_ptr()))
            {
                return entry.to.as_ptr().cast_mut();
            }
        }

        ptr::null_mut()
    }
}

/// Unescape the name in the translate dictionary table (`menu_unescape_name`).
fn menu_unescape_name(name: impl crate::strings::IntoCCharPtr) {
    let name = name.into_c_char_ptr().cast_mut();
    unsafe {
        let mut p = name;
        while !crate::memory::cstr_is_empty(p) && crate::memory::cstr_first_byte(p) != b'.' {
            if crate::memory::cstr_first_byte(p) == b'\\' {
                crate::memory::cstr_move(p, p.add(1));
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
    }
}

/// Isolate the menu name (`menu_translate_tab_and_shift`).
fn menu_translate_tab_and_shift(arg_start: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let arg_start = arg_start.into_c_char_ptr().cast_mut();
    unsafe {
        let mut arg = arg_start;

        while !crate::memory::cstr_is_empty(arg)
            && !ascii_iswhite(crate::memory::cstr_first_char(arg))
        {
            if (crate::memory::cstr_first_byte(arg) == b'\\' || *arg == CTRL_V_CONST)
                && crate::memory::cstr_byte_at(arg, 1) != 0
            {
                arg = arg.add(1);
            } else if crate::memory::cstr_prefix_bytes(arg, 5).eq_ignore_ascii_case(b"<TAB>") {
                *arg = TAB;
                crate::memory::cstr_move(arg.add(1), arg.add(5));
            }
            arg = arg.add(1);
        }
        if !crate::memory::cstr_is_empty(arg) {
            *arg = NUL;
            arg = arg.add(1);
        }
        skipwhite(arg)
    }
}

/// Get the information about a menu item in mode `modes` (`menuitem_getinfo`).
fn menuitem_getinfo(menu: &Vimmenu, modes: i32, dict: &mut TvDict) {
    {
        tv_dict_add_str(dict, c"name", menu.name.as_ptr());
        tv_dict_add_str(dict, c"display", menu.dname.as_ptr());
        if let Some(ref actext) = menu.actext {
            tv_dict_add_str(dict, c"accel", actext.as_ptr());
        }
        tv_dict_add_nr(dict, c"priority", i64::from(menu.priority));
        tv_dict_add_str(dict, c"modes", get_menu_mode_str(menu.modes));

        let mut buf = [0 as c_char; NUMBUFLEN];
        let buflen = utf_char2bytes(menu.mnemonic, buf.as_mut_ptr());
        buf[buflen as usize] = NUL;
        tv_dict_add_str_len(dict, c"shortcut", buf.as_ptr(), buflen);

        if menu.children.is_empty() {
            let mut bit = 0usize;
            while bit < MENU_MODES && ((1 << bit) & modes) == 0 {
                bit += 1;
            }

            if bit < MENU_MODES {
                if let Some(ref s) = menu.strings[bit] {
                    if s.as_bytes().is_empty() {
                        tv_dict_add_str(dict, c"rhs", c"<Nop>");
                    } else {
                        tv_dict_add_str(
                            dict,
                            c"rhs",
                            str2special_save(s.as_ptr(), false, K_FALSE).as_ptr(),
                        );
                    }
                }
                tv_dict_add_bool(
                    dict,
                    c"noremenu",
                    if menu.noremap[bit] == REMAP_NONE {
                        K_BOOL_VAR_TRUE_CONST
                    } else {
                        K_BOOL_VAR_FALSE_CONST
                    },
                );
                tv_dict_add_bool(
                    dict,
                    c"script",
                    if menu.noremap[bit] == REMAP_SCRIPT {
                        K_BOOL_VAR_TRUE_CONST
                    } else {
                        K_BOOL_VAR_FALSE_CONST
                    },
                );
                tv_dict_add_bool(
                    dict,
                    c"silent",
                    if menu.silent[bit] {
                        K_BOOL_VAR_TRUE_CONST
                    } else {
                        K_BOOL_VAR_FALSE_CONST
                    },
                );
                tv_dict_add_bool(
                    dict,
                    c"enabled",
                    if (menu.enabled & (1 << bit)) != 0 {
                        K_BOOL_VAR_TRUE_CONST
                    } else {
                        K_BOOL_VAR_FALSE_CONST
                    },
                );
            }
        } else {
            let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
            tv_dict_add_list(dict, c"submenus", l);
            for child in &menu.children {
                tv_list_append_string(&mut *l, child.dname.as_ptr(), -1);
            }
        }
    }
}

/// `menu_info()` function (`f_menu_info`).
pub fn f_menu_info(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let retdict = tv_dict_alloc_ret(rettv);

        let menu_name = tv_get_string_chk(&argvars[0]);
        if menu_name.is_null() {
            return;
        }

        let which = if argvars[1].v_type != VarType::VarUnknown {
            tv_get_string_chk(&argvars[1])
        } else {
            c"".as_ptr()
        };
        if which.is_null() {
            return;
        }

        let modes = get_menu_cmd_modes(
            which,
            crate::memory::cstr_first_byte(which) == b'!',
            None,
            None,
        );

        let mut menus = (&*get_root_menu(menu_name)).as_slice();
        if crate::memory::cstr_is_empty(menu_name) {
            let mut has_mode_match = false;
            for topmenu in menus {
                if (topmenu.modes & modes) != 0 {
                    has_mode_match = true;
                    break;
                }
            }
            if !menus.is_empty() && (menus[0].modes & modes) != 0 {
                let _ = has_mode_match;
                let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
                tv_dict_add_list(retdict, c"submenus", l);
                for topmenu in menus {
                    if !menu_is_hidden(topmenu.dname.as_ptr().cast_mut()) {
                        tv_list_append_string(&mut *l, topmenu.dname.as_ptr(), -1);
                    }
                }
            }
            return;
        }

        let mut saved_name_vec: Vec<c_char> = crate::memory::cstr_to_cchar_vec(menu_name);
        let mut found: *const Vimmenu = ptr::null();
        let mut name = saved_name_vec.as_mut_ptr();
        while !crate::memory::cstr_is_empty(name) {
            let p = menu_name_skip(name);
            found = ptr::null();
            for menu in menus {
                if menu_name_equal(name, menu) {
                    found = menu;
                    break;
                }
            }
            if found.is_null() || crate::memory::cstr_is_empty(p) {
                break;
            }
            menus = (*found).children.as_slice();
            name = p;
        }
        drop(saved_name_vec);

        if found.is_null() {
            return;
        }

        if ((*found).modes & modes) != 0 {
            menuitem_getinfo(&*found, modes, retdict);
        }
    }
}
