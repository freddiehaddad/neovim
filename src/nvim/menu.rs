//! Code for menus, used for the GUI and `'wildmenu'` (`src/nvim/menu.c`).

use crate::autocmd::{EVENT_MENUPOPUP, HLF_8, apply_autocmds};
use crate::charset::{Ctrl_V, ascii_isdigit, ascii_iswhite, getdigits_int, skipwhite};
use crate::cursor::{check_cursor, gchar_cursor, pos_T};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, VarType, dict_T, list_T, tv_dict_add_bool,
    tv_dict_add_dict, tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_add_str_len,
    tv_dict_alloc, tv_dict_alloc_ret, tv_get_string_chk, tv_list_alloc, tv_list_append_dict,
    tv_list_append_string, typval_T,
};
use crate::eval::vars::del_menutrans_vars;
use crate::ex_docmd::{
    Ctrl_C, Ctrl_G, Ctrl_O, ends_excmd, exarg_T, exec_normal_cmd, restore_current_state,
    save_current_state,
};
use crate::globals::*;
use crate::input::{REMAP_SCRIPT, ins_typebuf, tasave_T};
use crate::keycodes::{REPTERM_DO_LT, replace_termcodes};
use crate::mbyte::{utf_char2bytes, utfc_ptr2len};
use crate::memory::xstrlcpy;
use crate::message::{
    emsg, msg_outnum, msg_outtrans, msg_outtrans_special, msg_putchar, msg_puts, msg_puts_hl,
    msg_puts_title, semsg, str2special_save,
};
use crate::option_vars::*;
use crate::os::env::expand_T;
use crate::popupmenu::pum_show_popupmenu;
use crate::state::{
    MODE_ASKMORE, MODE_CMDLINE, MODE_HITRETURN, MODE_INSERT, MODE_LANGMAP, MODE_NORMAL,
    MODE_TERMINAL, MODE_VISUAL, get_real_state,
};
use crate::strings::{
    gettext, vim_strchr, };
use crate::ui::ui_call_update_menu;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

const Ctrl_BSL: c_char = 28;

const MENUDEPTH: usize = 10;
const TBUFFER_LEN: usize = 256;

const MENU_INDEX_INVALID: c_int = -1;
const MENU_INDEX_NORMAL: c_int = 0;
const MENU_INDEX_VISUAL: c_int = 1;
const MENU_INDEX_SELECT: c_int = 2;
const MENU_INDEX_OP_PENDING: c_int = 3;
const MENU_INDEX_INSERT: c_int = 4;
const MENU_INDEX_CMDLINE: c_int = 5;
const MENU_INDEX_TERMINAL: c_int = 6;
pub const MENU_MODES: usize = 8;

const MENU_NORMAL_MODE: c_int = 1 << MENU_INDEX_NORMAL;
const MENU_VISUAL_MODE: c_int = 1 << MENU_INDEX_VISUAL;
const MENU_SELECT_MODE: c_int = 1 << MENU_INDEX_SELECT;
const MENU_OP_PENDING_MODE: c_int = 1 << MENU_INDEX_OP_PENDING;
const MENU_INSERT_MODE: c_int = 1 << MENU_INDEX_INSERT;
const MENU_CMDLINE_MODE: c_int = 1 << MENU_INDEX_CMDLINE;
const MENU_TERMINAL_MODE: c_int = 1 << MENU_INDEX_TERMINAL;
const MENU_TIP_MODE: c_int = 1 << MENU_INDEX_TIP;

const MNU_HIDDEN_CHAR: c_char = b']' as c_char;

const MENU_MODE_CHARS: [&CStr; MENU_MODES] = [c"n", c"v", c"s", c"o", c"i", c"c", c"tl", c"t"];

const E_NOTSUBMENU: &CStr = c"E327: Part of menu-item path is not sub-menu";
const E_NOMENU: &CStr = c"E329: No menu \"%s\"";

#[repr(C)]
pub struct vimmenu_T {
    pub modes: c_int,
    pub enabled: c_int,
    pub name: std::ffi::CString,
    pub dname: std::ffi::CString,
    pub en_name: Option<std::ffi::CString>,
    pub en_dname: Option<std::ffi::CString>,
    pub mnemonic: c_int,
    pub actext: Option<std::ffi::CString>,
    pub priority: c_int,
    pub strings: [Option<std::ffi::CString>; MENU_MODES],
    pub noremap: [c_int; MENU_MODES],
    pub silent: [bool; MENU_MODES],
    pub children: Vec<vimmenu_T>,
}

impl vimmenu_T {
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

impl Default for vimmenu_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
struct menutrans_T {
    from: std::ffi::CString,
    from_noamp: std::ffi::CString,
    to: std::ffi::CString,
}

use crate::change::{MB_MAXCHAR, TAB};
use crate::charset::NUL;
use crate::cmdexpand::{
    EXPAND_MENUNAMES, EXPAND_MENUS, EXPAND_NOTHING, EXPAND_UNSUCCESSFUL, HLF_D,
};
use crate::digraph::{kBoolVarFalse, kBoolVarTrue};
use crate::eval::funcs::{MENU_ALL_MODES, MENU_INDEX_TIP};
use crate::eval::kListLenMayKnow;
use crate::extmark::{kFalse, kNone, kTrue};
use crate::mapping::{REMAP_NONE, REMAP_YES};
use crate::math::{FAIL, OK};
use crate::regexp::{MAXCOL, NUMBUFLEN};

#[repr(C)]
#[derive(Clone)]
pub struct save_state_T {
    pub save_msg_scroll: c_int,
    pub save_restart_edit: c_int,
    pub save_msg_didout: bool,
    pub save_State: c_int,
    pub save_finish_op: bool,
    pub save_opcount: c_int,
    pub save_reg_executing: c_int,
    pub save_pending_end_reg_executing: bool,
    pub tabuf: tasave_T,
}

impl save_state_T {
    pub const fn new() -> Self {
        Self {
            save_msg_scroll: 0,
            save_restart_edit: 0,
            save_msg_didout: false,
            save_State: 0,
            save_finish_op: false,
            save_opcount: 0,
            save_reg_executing: 0,
            save_pending_end_reg_executing: false,
            tabuf: tasave_T::new(),
        }
    }
}

struct MenuModuleState {
    MENUS_LOCKED: c_int,
    EXPAND_MENU: *const Vec<vimmenu_T>,
    EXPAND_MODES: c_int,
    EXPAND_EMENU: bool,
    GET_MENU_NAME_IDX: usize,
    GET_MENU_NAME_ADVANCE: bool,
    GET_MENU_NAMES_IDX: usize,
    GET_MENU_NAMES_ADVANCE: bool,
    GET_MENU_NAMES_TBUFFER: [c_char; TBUFFER_LEN],
    MENUTRANS_GA: Vec<menutrans_T>,
}

impl MenuModuleState {
    const fn new() -> Self {
        Self {
            MENUS_LOCKED: 0,
            EXPAND_MENU: ptr::null(),
            EXPAND_MODES: 0,
            EXPAND_EMENU: false,
            GET_MENU_NAME_IDX: 0,
            GET_MENU_NAME_ADVANCE: false,
            GET_MENU_NAMES_IDX: 0,
            GET_MENU_NAMES_ADVANCE: false,
            GET_MENU_NAMES_TBUFFER: [0; TBUFFER_LEN],
            MENUTRANS_GA: Vec::new(),
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
fn menu_is_winbar(name: *const c_char) -> bool {
    crate::memory::cstr_prefix_bytes(name, 6) == &b"WinBar"[..6]
}

fn get_root_menu(_name: *const c_char) -> *mut Vec<vimmenu_T> {
    &raw mut g_st().root_menu
}

/// If `menus_locked` is set then give an error and return true (`is_menus_locked`).
fn is_menus_locked() -> bool {
    if menu_st().MENUS_LOCKED > 0 {
            emsg(gettext(e_cannot_change_menus_while_listing.as_ptr()));
            return true;
        }
        false
}

/// Do the `:menu` command and relatives (`ex_menu`).
pub fn ex_menu(eap: &mut exarg_T) {
    unsafe {
        let mut noremap: c_int = 0;
        let mut silent = false;
        let mut unmenu = false;
        let mut pri_tab = [0 as c_int; MENUDEPTH + 1];
        let mut enable: c_int = kNone;

        let modes = get_menu_cmd_modes(
            eap.cmd,
            eap.forceit != 0,
            &raw mut noremap,
            &raw mut unmenu,
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
            while *arg != NUL && *arg != b' ' as c_char {
                if *arg == b'\\' as c_char {
                    ptr::copy(arg.add(1), arg, std::ffi::CStr::from_ptr(arg.add(1)).to_bytes_with_nul().len());
                }
                arg = arg.add(utfc_ptr2len(arg) as usize);
            }
            if *arg != NUL {
                *arg = NUL;
                arg = skipwhite(arg.add(1));
            }
        }

        let mut p = arg;
        while *p != NUL {
            if !ascii_isdigit(*p) && *p != b'.' as c_char {
                break;
            }
            p = p.add(1);
        }
        let mut i: usize;
        if ascii_iswhite(*p) {
            i = 0;
            while i < MENUDEPTH && !ascii_iswhite(*arg) {
                pri_tab[i] = getdigits_int(&raw mut arg, false, 0);
                if pri_tab[i] == 0 {
                    pri_tab[i] = 500;
                }
                if *arg == b'.' as c_char {
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

        if (crate::memory::cstr_prefix_bytes(arg, 6) == &b"enable"[..6]) && ascii_iswhite(*arg.add(6)) {
            enable = kTrue;
            arg = skipwhite(arg.add(6));
        } else if (crate::memory::cstr_prefix_bytes(arg, 7) == &b"disable"[..7]) && ascii_iswhite(*arg.add(7)) {
            enable = kFalse;
            arg = skipwhite(arg.add(7));
        }

        if *arg == NUL {
            show_menus(arg, modes);
            return;
        }

        let mut menu_path = arg;
        if *menu_path == b'.' as c_char {
            semsg!(gettext(e_invarg2.as_ptr()), menu_path);
            return;
        }

        let mut map_to = menu_translate_tab_and_shift(arg);

        if *map_to == NUL && !unmenu && enable == kNone {
            show_menus(menu_path, modes);
            return;
        } else if *map_to != NUL && (unmenu || enable != kNone) {
            semsg!(gettext(e_trailing_arg.as_ptr()), map_to);
            return;
        }

        let root_menu_ptr = get_root_menu(menu_path);

        if enable != kNone {
            if std::ffi::CStr::from_ptr(menu_path).to_bytes() == b"*" {
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

            if std::ffi::CStr::from_ptr(menu_path).to_bytes() == b"*" {
                menu_path = c"".as_ptr().cast_mut();
            }

            if menu_is_popup(menu_path) {
                for idx in 0..MENU_INDEX_TIP {
                    if (modes & (1 << idx)) != 0 {
                        let p = popup_mode_name(menu_path, idx as usize);
                        remove_menu(&mut *root_menu_ptr, p.as_ptr().cast_mut(), MENU_ALL_MODES, true);
                    }
                }
            }

            remove_menu(&mut *root_menu_ptr, menu_path, modes, false);
        } else {
            if is_menus_locked() {
                return;
            }

            let map_buf_owned = if std::ffi::CStr::from_ptr(map_to).to_bytes().eq_ignore_ascii_case(b"<nop>") {
                map_to = c"".as_ptr().cast_mut();
                None
            } else if (modes & MENU_TIP_MODE) == 0 {
                replace_termcodes(
                    map_to,
                    CStr::from_ptr(map_to).to_bytes().len(),
                    false,
                    0,
                    REPTERM_DO_LT,
                    ptr::null_mut(),
                    opt_vars().p_cpo,
                )
            } else {
                None
            };
            if let Some(ref mb) = map_buf_owned {
                map_to = mb.as_ptr().cast_mut();
            }

            let mut menuarg = vimmenu_T::new();
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
    menu_path: *const c_char,
    menuarg: &mut vimmenu_T,
    pri_tab: *const c_int,
    call_data: *const c_char,
) -> c_int {
    unsafe {
        let mut modes = menuarg.modes;
        let mut menu: *mut vimmenu_T = ptr::null_mut();
        let mut pri_idx: usize = 0;
        let mut old_modes: c_int = 0;

        let mut path_name_vec: Vec<c_char> = CStr::from_ptr(menu_path)
            .to_bytes_with_nul()
            .iter()
            .map(|&b| b as c_char)
            .collect();
        let path_name = path_name_vec.as_mut_ptr();
        let root_menu_ptr = get_root_menu(menu_path);
        let mut menup: *mut Vec<vimmenu_T> = root_menu_ptr;
        let mut ancestor_stack: Vec<(*mut Vec<vimmenu_T>, usize)> = Vec::new();
        let mut name = path_name;

        let cleanup_erret = |ancestor_stack: &[(*mut Vec<vimmenu_T>, usize)]| -> c_int {
            for &(mp, idx) in ancestor_stack.iter().rev() {
                if (&(*mp))[idx].children.is_empty() {
                    (*mp).remove(idx);
                } else {
                    break;
                }
            }
            FAIL
        };

        while *name != NUL {
            let next_name = menu_name_skip(name);
            let map_to = menutrans_lookup(name, CStr::from_ptr(name).to_bytes().len() as c_int);
            let en_name = if !map_to.is_null() {
                let orig = name;
                name = map_to;
                orig
            } else {
                ptr::null_mut()
            };
            let dname = menu_text(name, ptr::null_mut(), None);
            if dname.as_bytes().is_empty() {
                emsg(gettext(c"E792: Empty menu name".as_ptr()));
                return cleanup_erret(&ancestor_stack);
            }

            let mut lower_pri_idx: usize = 0;
            let mut found_idx: usize = 0;
            menu = ptr::null_mut();
            for (idx, m) in (*menup).iter_mut().enumerate() {
                if menu_name_equal(name, m) || menu_name_equal(dname.as_ptr(), m) {
                    if *next_name == NUL && !m.children.is_empty() {
                        if !g_st().sys_menu {
                            emsg(gettext(
                                c"E330: Menu path must not lead to a sub-menu".as_ptr(),
                            ));
                        }
                        return cleanup_erret(&ancestor_stack);
                    }
                    if *next_name != NUL && m.children.is_empty() {
                        if !g_st().sys_menu {
                            emsg(gettext(E_NOTSUBMENU.as_ptr()));
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
                if *next_name == NUL && ancestor_stack.is_empty() {
                    emsg(gettext(
                        c"E331: Must not add menu items directly to menu bar".as_ptr(),
                    ));
                    return cleanup_erret(&ancestor_stack);
                }

                if menu_is_separator(dname.as_ptr().cast_mut()) && *next_name != NUL {
                    emsg(gettext(
                        c"E332: Separator cannot be part of a menu path".as_ptr(),
                    ));
                    return cleanup_erret(&ancestor_stack);
                }

                let mut new_menu = vimmenu_T::new();
                new_menu.modes = modes;
                new_menu.enabled = MENU_ALL_MODES;
                new_menu.name = CStr::from_ptr(name).to_owned();
                new_menu.dname =
                    menu_text(name, &raw mut new_menu.mnemonic, Some(&mut new_menu.actext));
                if !en_name.is_null() {
                    new_menu.en_name = Some(CStr::from_ptr(en_name).to_owned());
                    new_menu.en_dname = Some(menu_text(en_name, ptr::null_mut(), None));
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
                Some(CStr::from_ptr(call_data).to_owned())
            };

            for i in 0..MENU_MODES {
                if (modes & (1 << i)) != 0 {
                    let mut c: c_char = 0;
                    let mut d: c_char = 0;
                    if amenu && !call_data.is_null() && *call_data != NUL {
                        match 1 << i {
                            MENU_VISUAL_MODE | MENU_SELECT_MODE | MENU_OP_PENDING_MODE
                            | MENU_CMDLINE_MODE => {
                                c = Ctrl_C;
                            }
                            MENU_INSERT_MODE => {
                                c = Ctrl_BSL;
                                d = Ctrl_O;
                            }
                            _ => {}
                        }
                    }

                    if c != 0 {
                        let call_bytes = CStr::from_ptr(call_data).to_bytes();
                        let mut bytes = Vec::with_capacity(call_bytes.len() + 4);
                        bytes.push(c as u8);
                        if d != 0 {
                            bytes.push(d as u8);
                        }
                        bytes.extend_from_slice(call_bytes);
                        if c == Ctrl_C {
                            bytes.push(Ctrl_BSL as u8);
                            bytes.push(Ctrl_G as u8);
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
        OK
    }
}

/// Set the (sub)menu with the given name to enabled or disabled (`menu_enable_recurse`).
fn menu_enable_recurse(
    menus: &mut [vimmenu_T],
    name: *mut c_char,
    modes: c_int,
    enable: bool,
) -> c_int {
    unsafe {
        if menus.is_empty() {
            return OK;
        }
        let p = menu_name_skip(name);

        let mut matched = false;
        for menu in menus.iter_mut() {
            if *name == NUL || *name == b'*' as c_char || menu_name_equal(name, menu) {
                matched = true;
                if *p != NUL {
                    if menu.children.is_empty() {
                        emsg(gettext(E_NOTSUBMENU.as_ptr()));
                        return FAIL;
                    }
                    if menu_enable_recurse(&mut menu.children, p, modes, enable) == FAIL {
                        return FAIL;
                    }
                } else if enable {
                    menu.enabled |= modes;
                } else {
                    menu.enabled &= !modes;
                }

                if *name != NUL && *name != b'*' as c_char {
                    break;
                }
            }
        }
        if *name != NUL && *name != b'*' as c_char && !matched {
            semsg!(gettext(E_NOMENU.as_ptr()), name);
            return FAIL;
        }

        OK
    }
}

/// Remove the (sub)menu with the given name from the menu hierarchy (`remove_menu`).
fn remove_menu(
    menup: &mut Vec<vimmenu_T>,
    name: *mut c_char,
    modes: c_int,
    silent: bool,
) -> c_int {
    unsafe {
        if menup.is_empty() {
            return OK;
        }
        let p = menu_name_skip(name);

        let mut idx: usize = 0;
        let mut found_menu = false;
        while idx < menup.len() {
            let menu: *mut vimmenu_T = &raw mut menup[idx];
            if *name == NUL || menu_name_equal(name, &*menu) {
                if *p != NUL && (*menu).children.is_empty() {
                    if !silent {
                        emsg(gettext(E_NOTSUBMENU.as_ptr()));
                    }
                    return FAIL;
                }
                if ((*menu).modes & modes) != 0 {
                    if remove_menu(&mut (*menu).children, p, modes, silent) == FAIL {
                        return FAIL;
                    }
                } else if *name != NUL {
                    if !silent {
                        emsg(gettext(e_menu_only_exists_in_another_mode.as_ptr()));
                    }
                    return FAIL;
                }

                if *name != NUL {
                    found_menu = true;
                    break;
                }

                (*menu).modes &= !modes;
                if (modes & MENU_TIP_MODE) != 0 {
                    (*menu).strings[MENU_INDEX_TIP as usize] = None;
                }
                if ((*menu).modes & MENU_ALL_MODES) == 0 {
                    menup.remove(idx);
                } else {
                    idx += 1;
                }
            } else {
                idx += 1;
            }
        }
        if *name != NUL {
            if !found_menu {
                if !silent {
                    semsg!(gettext(E_NOMENU.as_ptr()), name);
                }
                return FAIL;
            }

            let found = &raw mut menup[idx];
            (*found).modes &= !modes;
            for child in &(*found).children {
                (*found).modes |= child.modes;
            }
            if (modes & MENU_TIP_MODE) != 0 {
                (*found).strings[MENU_INDEX_TIP as usize] = None;
            }
            if ((*found).modes & MENU_ALL_MODES) == 0 {
                menup.remove(idx);
            }
        }

        OK
    }
}

/// Export menus recursively (`menu_get_recursive`).
fn menu_get_recursive(menu: *const vimmenu_T, modes: c_int) -> *mut dict_T {
    unsafe {
        if menu.is_null() || ((*menu).modes & modes) == 0 {
            return ptr::null_mut();
        }

        let dict = tv_dict_alloc();
        tv_dict_add_str(dict, c"name".as_ptr(), 4, (*menu).dname.as_ptr());
        tv_dict_add_nr(dict, c"priority".as_ptr(), 8, i64::from((*menu).priority));
        tv_dict_add_nr(
            dict,
            c"hidden".as_ptr(),
            6,
            i64::from(menu_is_hidden((*menu).dname.as_ptr().cast_mut())),
        );

        if (*menu).mnemonic != 0 {
            let mut buf = [0 as c_char; MB_MAXCHAR + 1];
            let buflen = utf_char2bytes((*menu).mnemonic, buf.as_mut_ptr());
            tv_dict_add_str_len(dict, c"shortcut".as_ptr(), 8, buf.as_ptr(), buflen);
        }

        if let Some(ref actext) = (*menu).actext {
            tv_dict_add_str(dict, c"actext".as_ptr(), 6, actext.as_ptr());
        }

        if ((*menu).modes & MENU_TIP_MODE) != 0
            && let Some(ref tip) = (*menu).strings[MENU_INDEX_TIP as usize]
        {
            tv_dict_add_str(dict, c"tooltip".as_ptr(), 7, tip.as_ptr());
        }

        if (*menu).children.is_empty() {
            let commands = tv_dict_alloc();
            tv_dict_add_dict(dict, c"mappings".as_ptr(), 8, commands);

            for (bit, mode_cstr) in MENU_MODE_CHARS.iter().enumerate() {
                if ((*menu).modes & modes & (1 << bit)) != 0 {
                    let impl_dict = tv_dict_alloc();
                    let s_ptr = (*menu).strings[bit]
                        .as_ref()
                        .map_or(c"".as_ptr(), |s| s.as_ptr());
                    tv_dict_add_str(
                        impl_dict,
                        c"rhs".as_ptr(),
                        3,
                        str2special_save(s_ptr, false, kFalse).as_ptr(),
                    );
                    tv_dict_add_nr(
                        impl_dict,
                        c"silent".as_ptr(),
                        6,
                        i64::from((*menu).silent[bit]),
                    );
                    tv_dict_add_nr(
                        impl_dict,
                        c"enabled".as_ptr(),
                        7,
                        if ((*menu).enabled & (1 << bit)) != 0 {
                            1
                        } else {
                            0
                        },
                    );
                    tv_dict_add_nr(
                        impl_dict,
                        c"noremap".as_ptr(),
                        7,
                        if (*menu).noremap[bit] == REMAP_NONE {
                            1
                        } else {
                            0
                        },
                    );
                    tv_dict_add_nr(
                        impl_dict,
                        c"sid".as_ptr(),
                        3,
                        if (*menu).noremap[bit] == REMAP_SCRIPT {
                            1
                        } else {
                            0
                        },
                    );
                    tv_dict_add_dict(
                        commands,
                        mode_cstr.as_ptr(),
                        mode_cstr.to_bytes().len(),
                        impl_dict,
                    );
                }
            }
        } else {
            let children_list = tv_list_alloc(kListLenMayKnow);
            for child in &(*menu).children {
                let d = menu_get_recursive(child, modes);
                if tv_dict_len(d) > 0 {
                    tv_list_append_dict(children_list, d);
                }
            }
            tv_dict_add_list(dict, c"submenus".as_ptr(), 8, children_list);
        }
        dict
    }
}

/// Export menus matching path `path_name` (`menu_get`).
pub fn menu_get(path_name: *mut c_char, modes: c_int, list: *mut list_T) -> bool {
    unsafe {
        let root = get_root_menu(path_name);
        if *path_name != NUL {
            let menu = find_menu(&mut *root, path_name, modes);
            if menu.is_null() {
                return false;
            }
            let d = menu_get_recursive(menu, modes);
            if !d.is_null() && tv_dict_len(d) > 0 {
                tv_list_append_dict(list, d);
            }
            return true;
        }
        for menu in &*root {
            let d = menu_get_recursive(menu, modes);
            if !d.is_null() && tv_dict_len(d) > 0 {
                tv_list_append_dict(list, d);
            }
        }
        true
    }
}

/// Find menu matching `path_name` and `modes` (`find_menu`).
fn find_menu(
    mut menus: &mut [vimmenu_T],
    path_name: *const c_char,
    modes: c_int,
) -> *mut vimmenu_T {
    unsafe {
        debug_assert!(*path_name != NUL);
        let mut saved_name_vec: Vec<c_char> = CStr::from_ptr(path_name)
            .to_bytes_with_nul()
            .iter()
            .map(|&b| b as c_char)
            .collect();
        let mut name = saved_name_vec.as_mut_ptr();
        let mut found: *mut vimmenu_T = ptr::null_mut();

        while *name != NUL {
            let p = menu_name_skip(name);
            found = ptr::null_mut();
            for menu in menus.iter_mut() {
                if menu_name_equal(name, menu) {
                    if *p != NUL && menu.children.is_empty() {
                        emsg(gettext(E_NOTSUBMENU.as_ptr()));
                        return ptr::null_mut();
                    } else if (menu.modes & modes) == 0 {
                        emsg(gettext(e_menu_only_exists_in_another_mode.as_ptr()));
                        return ptr::null_mut();
                    } else if *p == NUL {
                        return menu;
                    }
                    found = menu;
                    break;
                }
            }

            if found.is_null() {
                semsg!(gettext(E_NOMENU.as_ptr()), name);
                break;
            }
            name = p;
            debug_assert!(*name != NUL);
            menus = &mut (*found).children;
        }

        found
    }
}

/// Show the mapping associated with a menu item or hierarchy in a sub-menu (`show_menus`).
fn show_menus(path_name: *mut c_char, modes: c_int) -> c_int {
    unsafe {
        let mut menu: *mut vimmenu_T = ptr::null_mut();
        if *path_name != NUL {
            menu = find_menu(&mut *get_root_menu(path_name), path_name, modes);
            if menu.is_null() {
                return FAIL;
            }
        }

        menu_st().MENUS_LOCKED += 1;
        msg_puts_title(gettext(c"\n--- Menus ---".as_ptr()));
        show_menus_recursive(menu, modes, 0);
        menu_st().MENUS_LOCKED -= 1;
        OK
    }
}

/// Recursively show the mappings associated with the menus under the given one (`show_menus_recursive`).
fn show_menus_recursive(menu: *mut vimmenu_T, modes: c_int, mut depth: c_int) {
    unsafe {
        if !menu.is_null() && ((*menu).modes & modes) == 0 {
            return;
        }

        if !menu.is_null() {
            msg_putchar(b'\n' as c_int);
            if g_st().got_int {
                return;
            }
            for _ in 0..depth {
                msg_puts(c"  ".as_ptr());
            }
            if (*menu).priority != 0 {
                msg_outnum((*menu).priority);
                msg_puts(c" ".as_ptr());
            }
            msg_outtrans((*menu).name.as_ptr(), HLF_D, false);
        }

        if !menu.is_null() && (*menu).children.is_empty() {
            for (bit, mode_cstr) in MENU_MODE_CHARS.iter().enumerate() {
                if ((*menu).modes & modes & (1 << bit)) != 0 {
                    msg_putchar(b'\n' as c_int);
                    if g_st().got_int {
                        return;
                    }
                    for _ in 0..(depth + 2) {
                        msg_puts(c"  ".as_ptr());
                    }
                    msg_puts(mode_cstr.as_ptr());
                    if (*menu).noremap[bit] == REMAP_NONE {
                        msg_putchar(b'*' as c_int);
                    } else if (*menu).noremap[bit] == REMAP_SCRIPT {
                        msg_putchar(b'&' as c_int);
                    } else {
                        msg_putchar(b' ' as c_int);
                    }
                    if (*menu).silent[bit] {
                        msg_putchar(b's' as c_int);
                    } else {
                        msg_putchar(b' ' as c_int);
                    }
                    if ((*menu).modes & (*menu).enabled & (1 << bit)) == 0 {
                        msg_putchar(b'-' as c_int);
                    } else {
                        msg_putchar(b' ' as c_int);
                    }
                    msg_puts(c" ".as_ptr());
                    let s_bytes = (*menu).strings[bit]
                        .as_ref()
                        .map_or(&[][..], |s| s.as_bytes());
                    if s_bytes.is_empty() {
                        msg_puts_hl(c"<Nop>".as_ptr(), HLF_8, false);
                    } else {
                        msg_outtrans_special(
                            (*menu).strings[bit].as_ref().unwrap().as_ptr(),
                            false,
                            0,
                        );
                    }
                }
            }
        } else {
            let children = if menu.is_null() {
                depth -= 1;
                g_st().root_menu.as_mut_slice()
            } else {
                (*menu).children.as_mut_slice()
            };

            for child in children {
                if g_st().got_int {
                    break;
                }
                if !menu_is_hidden(child.dname.as_ptr().cast_mut()) {
                    show_menus_recursive(child, modes, depth + 1);
                }
            }
        }
    }
}

/// Work out what to complete when doing command line completion of menu names (`set_context_in_menu_cmd`).
pub fn set_context_in_menu_cmd(
    xp: &mut expand_T,
    cmd: *const c_char,
    mut arg: *mut c_char,
    forceit: bool,
) -> *mut c_char {
    unsafe {
        let mut path_name: *mut c_char = ptr::null_mut();
        let mut unmenu = false;

        xp.xp_context = EXPAND_UNSUCCESSFUL;

        let mut p = arg;
        while *p != NUL {
            if !ascii_isdigit(*p) && *p != b'.' as c_char {
                break;
            }
            p = p.add(1);
        }

        if !ascii_iswhite(*p) {
            if (crate::memory::cstr_prefix_bytes(arg, 6) == &b"enable"[..6])
                && (*arg.add(6) == NUL || ascii_iswhite(*arg.add(6)))
            {
                p = arg.add(6);
            } else if (crate::memory::cstr_prefix_bytes(arg, 7) == &b"disable"[..7])
                && (*arg.add(7) == NUL || ascii_iswhite(*arg.add(7)))
            {
                p = arg.add(7);
            } else {
                p = arg;
            }
        }

        while *p != NUL && ascii_iswhite(*p) {
            p = p.add(1);
        }

        let mut after_dot = p;
        arg = p;

        while *p != NUL && !ascii_iswhite(*p) {
            if (*p == b'\\' as c_char || *p == Ctrl_V) && *p.add(1) != NUL {
                p = p.add(1);
            } else if *p == b'.' as c_char {
                after_dot = p.add(1);
            }
            p = p.add(1);
        }

        let expand_menus =
            !((*cmd == b't' as c_char && *cmd.add(1) == b'e' as c_char) || *cmd == b'p' as c_char);
        menu_st().EXPAND_EMENU = *cmd == b'e' as c_char;
        if expand_menus && ascii_iswhite(*p) {
            return ptr::null_mut();
        }
        if *p == NUL {
            menu_st().EXPAND_MODES = get_menu_cmd_modes(cmd, forceit, ptr::null_mut(), &raw mut unmenu);
            if !unmenu {
                menu_st().EXPAND_MODES = MENU_ALL_MODES;
            }

            let mut menu_list: *const Vec<vimmenu_T> = &raw const g_st().root_menu;
            let mut path_name_vec: Vec<c_char>;
            if after_dot > arg {
                let path_len = after_dot.offset_from(arg) as usize;
                path_name_vec = vec![0; path_len];
                path_name = path_name_vec.as_mut_ptr();
                xstrlcpy(path_name, arg, path_len);
            }
            let mut name = path_name;
            while !name.is_null() && *name != NUL {
                p = menu_name_skip(name);
                let mut found: *const vimmenu_T = ptr::null();
                for menu in &*menu_list {
                    if menu_name_equal(name, menu) {
                        if (*p != NUL && menu.children.is_empty())
                            || (menu.modes & menu_st().EXPAND_MODES) == 0
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
            menu_st().EXPAND_MENU = menu_list;
        } else {
            xp.xp_context = EXPAND_NOTHING;
        }
        ptr::null_mut()
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of (sub)menus (not entries) (`get_menu_name`).
pub fn get_menu_name(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx == 0 {
            menu_st().GET_MENU_NAME_IDX = 0;
            menu_st().GET_MENU_NAME_ADVANCE = false;
        }

        if menu_st().EXPAND_MENU.is_null() {
            return ptr::null_mut();
        }
        let menus = (*menu_st().EXPAND_MENU).as_slice();

        while menu_st().GET_MENU_NAME_IDX < menus.len() {
            let menu = &menus[menu_st().GET_MENU_NAME_IDX];
            let dname_ptr = menu.dname.as_ptr().cast_mut();
            if menu_is_hidden(dname_ptr) || menu_is_separator(dname_ptr) || menu.children.is_empty()
            {
                menu_st().GET_MENU_NAME_IDX += 1;
            } else {
                break;
            }
        }

        if menu_st().GET_MENU_NAME_IDX >= menus.len() {
            return ptr::null_mut();
        }
        let menu = &menus[menu_st().GET_MENU_NAME_IDX];

        let str_ptr = if (menu.modes & menu_st().EXPAND_MODES) != 0 {
            if menu_st().GET_MENU_NAME_ADVANCE {
                menu.en_dname
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            } else {
                if menu.en_dname.is_none() {
                    menu_st().GET_MENU_NAME_ADVANCE = true;
                }
                menu.dname.as_ptr().cast_mut()
            }
        } else {
            c"".as_ptr().cast_mut()
        };

        if menu_st().GET_MENU_NAME_ADVANCE {
            menu_st().GET_MENU_NAME_IDX += 1;
        }

        menu_st().GET_MENU_NAME_ADVANCE = !menu_st().GET_MENU_NAME_ADVANCE;

        str_ptr
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of menus and menu entries (`get_menu_names`).
pub fn get_menu_names(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx == 0 {
            menu_st().GET_MENU_NAMES_IDX = 0;
            menu_st().GET_MENU_NAMES_ADVANCE = false;
        }

        if menu_st().EXPAND_MENU.is_null() {
            return ptr::null_mut();
        }
        let menus = (*menu_st().EXPAND_MENU).as_slice();

        while menu_st().GET_MENU_NAMES_IDX < menus.len() && {
            let dname = menus[menu_st().GET_MENU_NAMES_IDX].dname.as_ptr().cast_mut();
            let dlen = menus[menu_st().GET_MENU_NAMES_IDX].dname.as_bytes().len();
            menu_is_hidden(dname)
                || (menu_st().EXPAND_EMENU && menu_is_separator(dname))
                || (dlen > 0 && *dname.add(dlen - 1) == b'.' as c_char)
        } {
            menu_st().GET_MENU_NAMES_IDX += 1;
        }

        if menu_st().GET_MENU_NAMES_IDX >= menus.len() {
            return ptr::null_mut();
        }
        let menu = &menus[menu_st().GET_MENU_NAMES_IDX];

        let str_ptr = if (menu.modes & menu_st().EXPAND_MODES) != 0 {
            if !menu.children.is_empty() {
                if menu_st().GET_MENU_NAMES_ADVANCE {
                    xstrlcpy(
                        &raw mut menu_st().GET_MENU_NAMES_TBUFFER[0],
                        menu.en_dname.as_ref().map_or(c"".as_ptr(), |s| s.as_ptr()),
                        TBUFFER_LEN,
                    );
                } else {
                    xstrlcpy(
                        &raw mut menu_st().GET_MENU_NAMES_TBUFFER[0],
                        menu.dname.as_ptr(),
                        TBUFFER_LEN,
                    );
                    if menu.en_dname.is_none() {
                        menu_st().GET_MENU_NAMES_ADVANCE = true;
                    }
                }
                { let d = &raw mut menu_st().GET_MENU_NAMES_TBUFFER[0]; let s = c"\x01".as_ptr(); std::ptr::copy_nonoverlapping(s, d.add(std::ffi::CStr::from_ptr(d).to_bytes().len()), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                &raw mut menu_st().GET_MENU_NAMES_TBUFFER[0]
            } else if menu_st().GET_MENU_NAMES_ADVANCE {
                menu.en_dname
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            } else {
                if menu.en_dname.is_none() {
                    menu_st().GET_MENU_NAMES_ADVANCE = true;
                }
                menu.dname.as_ptr().cast_mut()
            }
        } else {
            c"".as_ptr().cast_mut()
        };

        if menu_st().GET_MENU_NAMES_ADVANCE {
            menu_st().GET_MENU_NAMES_IDX += 1;
        }

        menu_st().GET_MENU_NAMES_ADVANCE = !menu_st().GET_MENU_NAMES_ADVANCE;

        str_ptr
    }
}

/// Skip over this element of the menu path and return the start of the next element (`menu_name_skip`).
fn menu_name_skip(name: *mut c_char) -> *mut c_char {
    unsafe {
        let mut p = name;
        while *p != NUL && *p != b'.' as c_char {
            if *p == b'\\' as c_char || *p == Ctrl_V {
                ptr::copy(p.add(1), p, std::ffi::CStr::from_ptr(p.add(1)).to_bytes_with_nul().len());
                if *p == NUL {
                    break;
                }
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        if *p != NUL {
            *p = NUL;
            p = p.add(1);
        }
        p
    }
}

/// Return true when `name` matches with menu `menu` (`menu_name_equal`).
fn menu_name_equal(name: *const c_char, menu: &vimmenu_T) -> bool {
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

fn menu_namecmp(name: *const c_char, mname: *const c_char) -> bool {
    unsafe {
        let mut i: usize = 0;
        while *name.add(i) != NUL && *name.add(i) != TAB {
            if *name.add(i) != *mname.add(i) {
                break;
            }
            i += 1;
        }
        (*name.add(i) == NUL || *name.add(i) == TAB)
            && (*mname.add(i) == NUL || *mname.add(i) == TAB)
    }
}

/// Return the `MENU_MODES` specified by menu command `cmd` (`get_menu_cmd_modes`).
pub fn get_menu_cmd_modes(
    mut cmd: *const c_char,
    forceit: bool,
    noremap: *mut c_int,
    unmenu: *mut bool,
) -> c_int {
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
                if *cmd == b'l' as c_char {
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
            b'n' if *cmd != b'o' as c_char => MENU_NORMAL_MODE,
            _ => {
                cmd = cmd.sub(1);
                if forceit {
                    MENU_INSERT_MODE | MENU_CMDLINE_MODE
                } else {
                    MENU_NORMAL_MODE | MENU_VISUAL_MODE | MENU_SELECT_MODE | MENU_OP_PENDING_MODE
                }
            }
        };

        if !noremap.is_null() {
            *noremap = if *cmd == b'n' as c_char {
                REMAP_NONE
            } else {
                REMAP_YES
            };
        }
        if !unmenu.is_null() {
            *unmenu = *cmd == b'u' as c_char;
        }
        modes
    }
}

/// Return the string representation of the menu modes (`get_menu_mode_str`).
const fn get_menu_mode_str(modes: c_int) -> *const c_char {
    const ALL_A: c_int = MENU_INSERT_MODE
        | MENU_CMDLINE_MODE
        | MENU_NORMAL_MODE
        | MENU_VISUAL_MODE
        | MENU_SELECT_MODE
        | MENU_OP_PENDING_MODE;
    const ALL_DEF: c_int =
        MENU_NORMAL_MODE | MENU_VISUAL_MODE | MENU_SELECT_MODE | MENU_OP_PENDING_MODE;
    const ALL_BANG: c_int = MENU_INSERT_MODE | MENU_CMDLINE_MODE;
    const ALL_V: c_int = MENU_VISUAL_MODE | MENU_SELECT_MODE;

    if (modes & ALL_A) == ALL_A {
        c"a".as_ptr()
    } else if (modes & ALL_DEF) == ALL_DEF {
        c" ".as_ptr()
    } else if (modes & ALL_BANG) == ALL_BANG {
        c"!".as_ptr()
    } else if (modes & ALL_V) == ALL_V {
        c"v".as_ptr()
    } else if (modes & MENU_VISUAL_MODE) != 0 {
        c"x".as_ptr()
    } else if (modes & MENU_SELECT_MODE) != 0 {
        c"s".as_ptr()
    } else if (modes & MENU_OP_PENDING_MODE) != 0 {
        c"o".as_ptr()
    } else if (modes & MENU_INSERT_MODE) != 0 {
        c"i".as_ptr()
    } else if (modes & MENU_TERMINAL_MODE) != 0 {
        c"tl".as_ptr()
    } else if (modes & MENU_CMDLINE_MODE) != 0 {
        c"c".as_ptr()
    } else if (modes & MENU_NORMAL_MODE) != 0 {
        c"n".as_ptr()
    } else if (modes & MENU_TIP_MODE) != 0 {
        c"t".as_ptr()
    } else {
        c"".as_ptr()
    }
}

/// Modify a menu name starting with `"PopUp"` to include the mode character (`popup_mode_name`).
fn popup_mode_name(name: *mut c_char, idx: usize) -> std::ffi::CString {
    unsafe {
        let name_bytes = CStr::from_ptr(name).to_bytes();
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
    str: *const c_char,
    mnemonic: *mut c_int,
    actext: Option<&mut Option<std::ffi::CString>>,
) -> std::ffi::CString {
    unsafe {
        let mut p = vim_strchr(str, c_int::from(TAB as u8));
        let mut text_vec: Vec<c_char> = if !p.is_null() {
            if let Some(actext_out) = actext {
                *actext_out = Some(CStr::from_ptr(p.add(1)).to_owned());
            }
            let len = p.offset_from(str) as usize;
            let mut v = Vec::with_capacity(len + 1);
            v.extend_from_slice(std::slice::from_raw_parts(str, len));
            v.push(0);
            v
        } else {
            CStr::from_ptr(str)
                .to_bytes_with_nul()
                .iter()
                .map(|&b| b as c_char)
                .collect()
        };
        let text = text_vec.as_mut_ptr();

        p = text;
        while !p.is_null() {
            p = vim_strchr(p, b'&' as c_int);
            if !p.is_null() {
                if *p.add(1) == NUL {
                    break;
                }
                if !mnemonic.is_null() && *p.add(1) != b'&' as c_char {
                    *mnemonic = c_int::from(*p.add(1) as u8);
                }
                ptr::copy(p.add(1), p, std::ffi::CStr::from_ptr(p.add(1)).to_bytes_with_nul().len());
                p = p.add(1);
            }
        }
        CStr::from_ptr(text).to_owned()
    }
}

/// Return true if `name` can be a menu in the MenuBar (`menu_is_menubar`).
pub fn menu_is_menubar(name: *const c_char) -> bool {
    unsafe {
        !menu_is_popup(name)
            && !menu_is_toolbar(name)
            && !menu_is_winbar(name)
            && *name != MNU_HIDDEN_CHAR
    }
}

/// Return true if `name` is a popup menu name (`menu_is_popup`).
pub fn menu_is_popup(name: *const c_char) -> bool {
    crate::memory::cstr_prefix_bytes(name, 5) == &b"PopUp"[..5]
}

/// Return true if `name` is a toolbar menu name (`menu_is_toolbar`).
pub fn menu_is_toolbar(name: *const c_char) -> bool {
    crate::memory::cstr_prefix_bytes(name, 7) == &b"ToolBar"[..7]
}

/// Return true if the name is a menu separator identifier (`menu_is_separator`).
pub fn menu_is_separator(name: *mut c_char) -> bool {
    unsafe {
        let len = CStr::from_ptr(name).to_bytes().len();
        len > 0 && *name == b'-' as c_char && *name.add(len - 1) == b'-' as c_char
    }
}

/// True if a popup menu or starts with `MNU_HIDDEN_CHAR` (`menu_is_hidden`).
fn menu_is_hidden(name: *mut c_char) -> bool {
    unsafe { *name == MNU_HIDDEN_CHAR || (menu_is_popup(name) && *name.add(5) != NUL) }
}

fn get_menu_mode() -> c_int {
    if (g_st().State & MODE_TERMINAL) != 0 {
            return MENU_INDEX_TERMINAL;
        }
        if g_st().Visual.active {
            if g_st().Visual.select {
                return MENU_INDEX_SELECT;
            }
            return MENU_INDEX_VISUAL;
        }
        if (g_st().State & MODE_INSERT) != 0 {
            return MENU_INDEX_INSERT;
        }
        if (g_st().State & MODE_CMDLINE) != 0 || g_st().State == MODE_ASKMORE || g_st().State == MODE_HITRETURN {
            return MENU_INDEX_CMDLINE;
        }
        if g_st().finish_op {
            return MENU_INDEX_OP_PENDING;
        }
        if (g_st().State & MODE_NORMAL) != 0 {
            return MENU_INDEX_NORMAL;
        }
        if (g_st().State & MODE_LANGMAP) != 0 {
            return MENU_INDEX_INSERT;
        }
        MENU_INDEX_INVALID
}

pub fn get_menu_mode_flag() -> c_int {
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

        let mut menu: *mut vimmenu_T = ptr::null_mut();
        let root_ptr = &raw mut g_st().root_menu;
        for m in &mut *root_ptr {
            if (crate::memory::cstr_prefix_bytes(c"PopUp".as_ptr(), 5) == crate::memory::cstr_prefix_bytes(m.name.as_ptr(), 5))
                && (crate::memory::cstr_prefix_bytes(m.name.as_ptr().add(5), mode_len) == crate::memory::cstr_prefix_bytes(mode, mode_len))
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
pub fn execute_menu(eap: *const exarg_T, menu: &mut vimmenu_T, mode_idx: c_int) {
    unsafe {
        let mut idx = mode_idx;

        if idx < 0 {
            if (g_st().State & MODE_TERMINAL) != 0 {
                idx = MENU_INDEX_TERMINAL;
            } else if (g_st().State & MODE_CMDLINE) != 0 {
                idx = MENU_INDEX_CMDLINE;
            } else if (get_real_state() & MODE_VISUAL) != 0 {
                idx = MENU_INDEX_VISUAL;
            } else if ((g_st().State & MODE_INSERT) != 0 || g_st().restart_edit != 0) && g_st().current_sctx.sc_sid == 0
            {
                idx = MENU_INDEX_INSERT;
            } else if !eap.is_null() && (*eap).addr_count != 0 {
                idx = MENU_INDEX_VISUAL;

                let tpos: pos_T;
                if (*g_st().curbuf).b_visual.vi_start.lnum == (*eap).line1
                    && (*g_st().curbuf).b_visual.vi_end.lnum == (*eap).line2
                {
                    g_st().Visual.mode = (*g_st().curbuf).b_visual.vi_mode;
                    tpos = (*g_st().curbuf).b_visual.vi_end;
                    (**curwin.as_ptr()).w_cursor = (*g_st().curbuf).b_visual.vi_start;
                    (**curwin.as_ptr()).w_curswant = (*g_st().curbuf).b_visual.vi_curswant;
                } else {
                    g_st().Visual.mode = b'V' as c_int;
                    (**curwin.as_ptr()).w_cursor.lnum = (*eap).line1;
                    (**curwin.as_ptr()).w_cursor.col = 1;
                    tpos = pos_T {
                        lnum: (*eap).line2,
                        col: MAXCOL,
                        coladd: 0,
                    };
                }

                g_st().Visual.active = true;
                g_st().Visual.reselect = 1;
                check_cursor(*curwin.as_ptr());
                g_st().Visual.start = (**curwin.as_ptr()).w_cursor;
                (**curwin.as_ptr()).w_cursor = tpos;

                check_cursor(*curwin.as_ptr());

                if *opt_vars().p_sel == b'e' as c_char && gchar_cursor() != c_int::from(NUL) {
                    (**curwin.as_ptr()).w_cursor.col += 1;
                }
            }
        }

        if idx == MENU_INDEX_INVALID || eap.is_null() {
            idx = MENU_INDEX_NORMAL;
        }

        let u_idx = idx as usize;
        if let Some(ref cmd_str) = menu.strings[u_idx]
            && (menu.modes & (1 << idx)) != 0
        {
            let cmd_ptr = cmd_str.as_ptr().cast_mut();
            if eap.is_null() || g_st().current_sctx.sc_sid != 0 {
                let mut save_state = save_state_T::new();
                g_st().ex_normal_busy += 1;
                save_current_state(&mut save_state);
                exec_normal_cmd(cmd_ptr, menu.noremap[u_idx], menu.silent[u_idx]);
                restore_current_state(&mut save_state);
                g_st().ex_normal_busy -= 1;
            } else {
                ins_typebuf(
                    cmd_ptr,
                    menu.noremap[u_idx],
                    0,
                    true,
                    menu.silent[u_idx],
                );
            }
        } else if !eap.is_null() {
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
                gettext(c"E335: Menu not defined for %s mode".as_ptr()),
                mode.as_ptr(),
            );
        }
    }
}

/// Lookup a menu by the descriptor name e.g. `"File.New"` (`menu_getbyname`).
fn menu_getbyname(name_arg: *mut c_char) -> *mut vimmenu_T {
    unsafe {
        let mut saved_name_vec: Vec<c_char> = CStr::from_ptr(name_arg)
            .to_bytes_with_nul()
            .iter()
            .map(|&b| b as c_char)
            .collect();
        let saved_name = saved_name_vec.as_mut_ptr();
        let mut menus = (&mut *get_root_menu(saved_name)).as_mut_slice();
        let mut name = saved_name;
        let mut gave_emsg = false;
        let mut found: *mut vimmenu_T = ptr::null_mut();
        while *name != NUL {
            let p = menu_name_skip(name);
            found = ptr::null_mut();

            for menu in menus.iter_mut() {
                if menu_name_equal(name, menu) {
                    if *p == NUL && !menu.children.is_empty() {
                        emsg(gettext(
                            c"E333: Menu path must lead to a menu item".as_ptr(),
                        ));
                        gave_emsg = true;
                    } else if *p != NUL && menu.children.is_empty() {
                        emsg(gettext(E_NOTSUBMENU.as_ptr()));
                    } else {
                        found = menu;
                    }
                    break;
                }
            }
            if found.is_null() || *p == NUL {
                break;
            }
            menus = (*found).children.as_mut_slice();
            name = p;
        }
        drop(saved_name_vec);
        if found.is_null() {
            if !gave_emsg {
                semsg!(gettext(c"E334: Menu not found: %s".as_ptr()), name_arg);
            }
            return ptr::null_mut();
        }

        found
    }
}

/// `:emenu` (`ex_emenu`).
pub fn ex_emenu(eap: &mut exarg_T) {
    unsafe {
        let mut arg = eap.arg;
        let mut mode_idx = MENU_INDEX_INVALID;

        if *arg != NUL && ascii_iswhite(*arg.add(1)) {
            match *arg as u8 {
                b'n' => mode_idx = MENU_INDEX_NORMAL,
                b'v' => mode_idx = MENU_INDEX_VISUAL,
                b's' => mode_idx = MENU_INDEX_SELECT,
                b'o' => mode_idx = MENU_INDEX_OP_PENDING,
                b't' => mode_idx = MENU_INDEX_TERMINAL,
                b'i' => mode_idx = MENU_INDEX_INSERT,
                b'c' => mode_idx = MENU_INDEX_CMDLINE,
                _ => {
                    semsg!(gettext(e_invarg2.as_ptr()), arg);
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
pub fn menu_find(path_name: *const c_char) -> *mut vimmenu_T {
    unsafe {
        let mut menus = (&mut *get_root_menu(path_name)).as_mut_slice();
        let mut saved_name_vec: Vec<c_char> = CStr::from_ptr(path_name)
            .to_bytes_with_nul()
            .iter()
            .map(|&b| b as c_char)
            .collect();
        let mut name = saved_name_vec.as_mut_ptr();
        let mut found: *mut vimmenu_T = ptr::null_mut();
        while *name != NUL {
            let p = menu_name_skip(name);
            found = ptr::null_mut();

            for menu in menus.iter_mut() {
                if menu_name_equal(name, menu) {
                    if menu.children.is_empty() {
                        if *p == NUL {
                            emsg(gettext(c"E336: Menu path must lead to a sub-menu".as_ptr()));
                        } else {
                            emsg(gettext(E_NOTSUBMENU.as_ptr()));
                        }
                        return ptr::null_mut();
                    }
                    if *p == NUL {
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
            emsg(gettext(c"E337: Menu not found - check menu names".as_ptr()));
        }
        found
    }
}

/// `:menutrans` (`ex_menutranslate`).
pub fn ex_menutranslate(eap: &mut exarg_T) {
    unsafe {
        let mut arg = eap.arg;
        let mt_ptr = &raw mut menu_st().MENUTRANS_GA;
        let menutrans = &mut *mt_ptr;

        if (crate::memory::cstr_prefix_bytes(arg, 5) == &b"clear"[..5])
            && ends_excmd(c_int::from(*skipwhite(arg.add(5)) as u8)) != 0
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
                emsg(gettext(e_invarg.as_ptr()));
            } else {
                let mut from_vec: Vec<c_char> = CStr::from_ptr(from_arg)
                    .to_bytes_with_nul()
                    .iter()
                    .map(|&b| b as c_char)
                    .collect();
                let from = from_vec.as_mut_ptr();
                let from_noamp_cstr = menu_text(from, ptr::null_mut(), None);
                let to_len = arg.offset_from(to_arg) as usize;
                let mut to_vec: Vec<c_char> = Vec::with_capacity(to_len + 1);
                to_vec.extend_from_slice(std::slice::from_raw_parts(to_arg, to_len));
                to_vec.push(0);
                let to = to_vec.as_mut_ptr();
                menu_translate_tab_and_shift(from);
                menu_translate_tab_and_shift(to);
                menu_unescape_name(from);
                menu_unescape_name(to);
                let from_cstr = CStr::from_ptr(from).to_owned();
                let to_cstr = CStr::from_ptr(to).to_owned();
                menutrans.push(menutrans_T {
                    from: from_cstr,
                    from_noamp: from_noamp_cstr,
                    to: to_cstr,
                });
            }
        }
    }
}

/// Find the character just after one part of a menu name (`menu_skip_part`).
fn menu_skip_part(mut p: *mut c_char) -> *mut c_char {
    unsafe {
        while *p != NUL && *p != b'.' as c_char && !ascii_iswhite(*p) {
            if (*p == b'\\' as c_char || *p == Ctrl_V) && *p.add(1) != NUL {
                p = p.add(1);
            }
            p = p.add(1);
        }
        p
    }
}

/// Lookup part of a menu name in the translations (`menutrans_lookup`).
fn menutrans_lookup(name: *mut c_char, len: c_int) -> *mut c_char {
    unsafe {
        let mt_ptr = &raw const menu_st().MENUTRANS_GA;
        let menutrans = &*mt_ptr;
        let u_len = len as usize;

        for entry in menutrans {
            let from_ptr = entry.from.as_ptr();
            if crate::memory::cstr_prefix_bytes(name, u_len).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(from_ptr, u_len)) && *from_ptr.add(u_len) == NUL {
                return entry.to.as_ptr().cast_mut();
            }
        }

        let c = *name.add(u_len);
        *name.add(u_len) = NUL;
        let dname = menu_text(name, ptr::null_mut(), None);
        *name.add(u_len) = c;
        for entry in menutrans {
            if std::ffi::CStr::from_ptr(dname.as_ptr()).to_bytes().eq_ignore_ascii_case(std::ffi::CStr::from_ptr(entry.from_noamp.as_ptr()).to_bytes()) {
                return entry.to.as_ptr().cast_mut();
            }
        }

        ptr::null_mut()
    }
}

/// Unescape the name in the translate dictionary table (`menu_unescape_name`).
fn menu_unescape_name(name: *mut c_char) {
    unsafe {
        let mut p = name;
        while *p != NUL && *p != b'.' as c_char {
            if *p == b'\\' as c_char {
                ptr::copy(p.add(1), p, std::ffi::CStr::from_ptr(p.add(1)).to_bytes_with_nul().len());
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
    }
}

/// Isolate the menu name (`menu_translate_tab_and_shift`).
fn menu_translate_tab_and_shift(arg_start: *mut c_char) -> *mut c_char {
    unsafe {
        let mut arg = arg_start;

        while *arg != NUL && !ascii_iswhite(*arg) {
            if (*arg == b'\\' as c_char || *arg == Ctrl_V) && *arg.add(1) != NUL {
                arg = arg.add(1);
            } else if crate::memory::cstr_prefix_bytes(arg, 5).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"<TAB>".as_ptr(), 5)) {
                *arg = TAB;
                ptr::copy(arg.add(5), arg.add(1), std::ffi::CStr::from_ptr(arg.add(5)).to_bytes_with_nul().len());
            }
            arg = arg.add(1);
        }
        if *arg != NUL {
            *arg = NUL;
            arg = arg.add(1);
        }
        skipwhite(arg)
    }
}

/// Get the information about a menu item in mode `modes` (`menuitem_getinfo`).
fn menuitem_getinfo(menu: &vimmenu_T, modes: c_int, dict: *mut dict_T) {
    unsafe {
        tv_dict_add_str(dict, c"name".as_ptr(), 4, menu.name.as_ptr());
        tv_dict_add_str(dict, c"display".as_ptr(), 7, menu.dname.as_ptr());
        if let Some(ref actext) = menu.actext {
            tv_dict_add_str(dict, c"accel".as_ptr(), 5, actext.as_ptr());
        }
        tv_dict_add_nr(dict, c"priority".as_ptr(), 8, i64::from(menu.priority));
        tv_dict_add_str(dict, c"modes".as_ptr(), 5, get_menu_mode_str(menu.modes));

        let mut buf = [0 as c_char; NUMBUFLEN];
        let buflen = utf_char2bytes(menu.mnemonic, buf.as_mut_ptr());
        buf[buflen as usize] = NUL;
        tv_dict_add_str_len(dict, c"shortcut".as_ptr(), 8, buf.as_ptr(), buflen);

        if menu.children.is_empty() {
            let mut bit = 0usize;
            while bit < MENU_MODES && ((1 << bit) & modes) == 0 {
                bit += 1;
            }

            if bit < MENU_MODES {
                if let Some(ref s) = menu.strings[bit] {
                    if s.as_bytes().is_empty() {
                        tv_dict_add_str(dict, c"rhs".as_ptr(), 3, c"<Nop>".as_ptr());
                    } else {
                        tv_dict_add_str(
                            dict,
                            c"rhs".as_ptr(),
                            3,
                            str2special_save(s.as_ptr(), false, kFalse).as_ptr(),
                        );
                    }
                }
                tv_dict_add_bool(
                    dict,
                    c"noremenu".as_ptr(),
                    8,
                    if menu.noremap[bit] == REMAP_NONE {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
                tv_dict_add_bool(
                    dict,
                    c"script".as_ptr(),
                    6,
                    if menu.noremap[bit] == REMAP_SCRIPT {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
                tv_dict_add_bool(
                    dict,
                    c"silent".as_ptr(),
                    6,
                    if menu.silent[bit] {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
                tv_dict_add_bool(
                    dict,
                    c"enabled".as_ptr(),
                    7,
                    if (menu.enabled & (1 << bit)) != 0 {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
            }
        } else {
            let l = tv_list_alloc(kListLenMayKnow);
            tv_dict_add_list(dict, c"submenus".as_ptr(), 8, l);
            for child in &menu.children {
                tv_list_append_string(l, child.dname.as_ptr(), -1);
            }
        }
    }
}

/// `menu_info()` function (`f_menu_info`).
pub fn f_menu_info(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    let argvars = argvars.as_mut_ptr();

    unsafe {
        tv_dict_alloc_ret(rettv);
        let retdict = rettv.vval.v_dict();

        let menu_name = tv_get_string_chk(argvars);
        if menu_name.is_null() {
            return;
        }

        let which = if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            tv_get_string_chk(argvars.add(1))
        } else {
            c"".as_ptr()
        };
        if which.is_null() {
            return;
        }

        let modes = get_menu_cmd_modes(
            which,
            *which == b'!' as c_char,
            ptr::null_mut(),
            ptr::null_mut(),
        );

        let mut menus = (&*get_root_menu(menu_name)).as_slice();
        if *menu_name == NUL {
            let mut has_mode_match = false;
            for topmenu in menus {
                if (topmenu.modes & modes) != 0 {
                    has_mode_match = true;
                    break;
                }
            }
            if !menus.is_empty() && (menus[0].modes & modes) != 0 {
                let _ = has_mode_match;
                let l = tv_list_alloc(kListLenMayKnow);
                tv_dict_add_list(retdict, c"submenus".as_ptr(), 8, l);
                for topmenu in menus {
                    if !menu_is_hidden(topmenu.dname.as_ptr().cast_mut()) {
                        tv_list_append_string(l, topmenu.dname.as_ptr(), -1);
                    }
                }
            }
            return;
        }

        let mut saved_name_vec: Vec<c_char> = CStr::from_ptr(menu_name)
            .to_bytes_with_nul()
            .iter()
            .map(|&b| b as c_char)
            .collect();
        let mut found: *const vimmenu_T = ptr::null();
        let mut name = saved_name_vec.as_mut_ptr();
        while *name != NUL {
            let p = menu_name_skip(name);
            found = ptr::null();
            for menu in menus {
                if menu_name_equal(name, menu) {
                    found = menu;
                    break;
                }
            }
            if found.is_null() || *p == NUL {
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
