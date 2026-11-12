//! Code for menus, used for the GUI and `'wildmenu'` (`src/nvim/menu.c`).
#![forbid(unsafe_code)]

use std::ffi::CString;

use crate::autocmd::{EVENT_MENUPOPUP, HLF_8, apply_autocmds};
use crate::charset::{CTRL_V_CONST, ascii_isdigit, ascii_iswhite, getdigits_int};
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
use crate::mbyte::utf_char2bytes;
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
use crate::strings::gettext;
use crate::ui::ui_call_update_menu;
use std::ffi::CStr;

const CTRL_BSL_CONST: u8 = 28;

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

const MNU_HIDDEN_CHAR: u8 = b']';

const MENU_MODE_CHARS: [&CStr; MENU_MODES] = [c"n", c"v", c"s", c"o", c"i", c"c", c"tl", c"t"];

const E_NOTSUBMENU: &CStr = c"E327: Part of menu-item path is not sub-menu";
const E_NOMENU: &CStr = c"E329: No menu \"%s\"";

#[repr(C)]
#[derive(Clone)]
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

struct MenuCopyState {
    menus_locked: std::cell::Cell<i32>,
    expand_modes: std::cell::Cell<i32>,
    expand_emenu: std::cell::Cell<bool>,
    get_menu_name_idx: std::cell::Cell<usize>,
    get_menu_name_advance: std::cell::Cell<bool>,
    get_menu_names_idx: std::cell::Cell<usize>,
    get_menu_names_advance: std::cell::Cell<bool>,
}

impl MenuCopyState {
    const fn new() -> Self {
        Self {
            menus_locked: std::cell::Cell::new(0),
            expand_modes: std::cell::Cell::new(0),
            expand_emenu: std::cell::Cell::new(false),
            get_menu_name_idx: std::cell::Cell::new(0),
            get_menu_name_advance: std::cell::Cell::new(false),
            get_menu_names_idx: std::cell::Cell::new(0),
            get_menu_names_advance: std::cell::Cell::new(false),
        }
    }
}

thread_local! {
    static MENU_STATE: MenuCopyState = const { MenuCopyState::new() };
    static EXPAND_MENU_PATH: std::cell::RefCell<Option<Vec<usize>>> = const {
        std::cell::RefCell::new(None)
    };
    static MENUTRANS_GA: std::cell::RefCell<Vec<Menutrans>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

fn expand_menu_slice() -> Option<Vec<Vimmenu>> {
    EXPAND_MENU_PATH.with(|cell| {
        let path = cell.borrow();
        let path = path.as_ref()?;
        let root = g_st().root_menu.borrow();
        let mut list: &[Vimmenu] = &root;
        for &idx in path {
            list = &list.get(idx)?.children;
        }
        Some(list.to_vec())
    })
}

use crate::eval::typval::tv_dict_len;

/// Return true if `name` is a window toolbar menu name (`menu_is_winbar`).
fn menu_is_winbar(name: &[u8]) -> bool {
    name.starts_with(b"WinBar")
}

/// If `menus_locked` is set then give an error and return true (`is_menus_locked`).
fn is_menus_locked() -> bool {
    if MENU_STATE.with(|s| s.menus_locked.get()) > 0 {
        emsg(gettext(E_CANNOT_CHANGE_MENUS_WHILE_LISTING));
        return true;
    }
    false
}

/// Do the `:menu` command and relatives (`ex_menu`).
pub fn ex_menu(eap: &mut Exarg) {
    let mut noremap: i32 = 0;
    let mut silent = false;
    let mut unmenu = false;
    let mut pri_tab = [0_i32; MENUDEPTH + 1];
    let mut enable: i32 = K_NONE;

    let modes = get_menu_cmd_modes(
        eap.cmd.as_cstr(),
        eap.forceit,
        Some(&mut noremap),
        Some(&mut unmenu),
    );
    let mut arg = eap.arg.clone();

    loop {
        let arg_bytes = arg.as_bytes();
        if arg_bytes.starts_with(b"<script>") {
            noremap = REMAP_SCRIPT;
            arg = arg.wrapping_add(8).skipwhite();
            continue;
        }
        if arg_bytes.starts_with(b"<silent>") {
            silent = true;
            arg = arg.wrapping_add(8).skipwhite();
            continue;
        }
        if arg_bytes.starts_with(b"<special>") {
            arg = arg.wrapping_add(9).skipwhite();
            continue;
        }
        break;
    }

    if arg.as_bytes().starts_with(b"icon=") {
        arg = arg.wrapping_add(5);
        let mut arg_len = arg.len();
        let arg_slice = arg.cstr_buf_mut();
        let mut idx = 0;
        while idx < arg_len && arg_slice[idx] != b' ' {
            if arg_slice[idx] == b'\\' {
                arg_slice.copy_within(idx + 1..=arg_len, idx);
                arg_len -= 1;
            }
            if idx < arg_len {
                idx += crate::mbyte::utfc_slice2len(&arg_slice[idx..arg_len]);
            }
        }
        if idx < arg_len {
            arg_slice[idx] = 0;
            idx += 1;
            let rem = crate::charset::skipwhite_slice(&arg_slice[idx..arg_len]);
            idx = arg_len - rem.len();
        }
        arg = arg.wrapping_add(idx);
    }

    let p_len = arg
        .as_bytes()
        .iter()
        .take_while(|&&b| ascii_isdigit(b) || b == b'.')
        .count();
    let mut i: usize;
    if ascii_iswhite(arg.byte_at(p_len)) {
        let arg_bytes = arg.as_bytes().to_vec();
        let mut rem = arg_bytes.as_slice();
        i = 0;
        while i < MENUDEPTH && rem.first().is_some_and(|&b| !ascii_iswhite(i32::from(b))) {
            pri_tab[i] = getdigits_int(&mut rem, false, 0);
            if pri_tab[i] == 0 {
                pri_tab[i] = 500;
            }
            if rem.first() == Some(&b'.') {
                rem = &rem[1..];
            }
            i += 1;
        }
        arg = arg.wrapping_add(arg_bytes.len() - rem.len()).skipwhite();
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

    let arg_bytes = arg.as_bytes();
    if arg_bytes.starts_with(b"enable")
        && arg_bytes
            .get(6)
            .is_some_and(|&b| ascii_iswhite(i32::from(b)))
    {
        enable = K_TRUE;
        arg = arg.wrapping_add(6).skipwhite();
    } else if arg_bytes.starts_with(b"disable")
        && arg_bytes
            .get(7)
            .is_some_and(|&b| ascii_iswhite(i32::from(b)))
    {
        enable = K_FALSE;
        arg = arg.wrapping_add(7).skipwhite();
    }

    if arg.is_empty() {
        show_menus(arg.as_cstr(), modes);
        return;
    }

    if arg.first_byte() == b'.' {
        semsg!(gettext(E_INVARG2), &arg);
        return;
    }

    let map_to_off = menu_translate_tab_and_shift(arg.cstr_buf_mut());
    let map_to_slice = arg.wrapping_add(map_to_off);
    let map_to_empty = map_to_slice.is_empty();
    let mut menu_path = arg.as_cstr();

    if map_to_empty && !unmenu && enable == K_NONE {
        show_menus(menu_path, modes);
        return;
    } else if !map_to_empty && (unmenu || enable != K_NONE) {
        semsg!(gettext(E_TRAILING_ARG), &map_to_slice);
        return;
    }

    if enable != K_NONE {
        if menu_path.to_bytes() == b"*" {
            menu_path = c"";
        }

        if menu_is_popup(menu_path.to_bytes()) {
            for idx in 0..MENU_INDEX_TIP {
                if (modes & (1 << idx)) != 0 {
                    let p = popup_mode_name(menu_path.to_bytes(), idx as usize);
                    menu_enable_recurse(
                        &mut g_st().root_menu.borrow_mut(),
                        p.as_c_str(),
                        MENU_ALL_MODES,
                        enable != 0,
                    );
                }
            }
        }
        menu_enable_recurse(
            &mut g_st().root_menu.borrow_mut(),
            menu_path,
            modes,
            enable != 0,
        );
    } else if unmenu {
        if is_menus_locked() {
            return;
        }

        if menu_path.to_bytes() == b"*" {
            menu_path = c"";
        }

        if menu_is_popup(menu_path.to_bytes()) {
            for idx in 0..MENU_INDEX_TIP {
                if (modes & (1 << idx)) != 0 {
                    let p = popup_mode_name(menu_path.to_bytes(), idx as usize);
                    remove_menu(
                        &mut g_st().root_menu.borrow_mut(),
                        p.as_c_str(),
                        MENU_ALL_MODES,
                        true,
                    );
                }
            }
        }

        remove_menu(&mut g_st().root_menu.borrow_mut(), menu_path, modes, false);
    } else {
        if is_menus_locked() {
            return;
        }

        let map_buf_owned: Option<CString>;
        let map_to_cstr: Option<&CStr> = if map_to_slice.as_bytes().eq_ignore_ascii_case(b"<nop>") {
            Some(c"")
        } else if (modes & MENU_TIP_MODE) == 0 {
            map_buf_owned = replace_termcodes(
                map_to_slice.as_bytes(),
                false,
                0,
                REPTERM_DO_LT,
                None,
                opt_vars().p_cpo.as_bytes(),
            );
            match map_buf_owned {
                Some(ref mb) => Some(mb.as_c_str()),
                None => map_to_slice.as_opt_cstr(),
            }
        } else {
            map_to_slice.as_opt_cstr()
        };

        let mut menuarg = Vimmenu::new();
        menuarg.modes = modes;
        menuarg.noremap[0] = noremap;
        menuarg.silent[0] = silent;
        add_menu_path(menu_path, &mut menuarg, &pri_tab, map_to_cstr);

        if menu_is_popup(menu_path.to_bytes()) {
            for idx in 0..MENU_INDEX_TIP {
                if (modes & (1 << idx)) != 0 {
                    let p = popup_mode_name(menu_path.to_bytes(), idx as usize);
                    menuarg.modes = modes;
                    add_menu_path(p.as_c_str(), &mut menuarg, &pri_tab, map_to_cstr);
                }
            }
        }
    }

    ui_call_update_menu();
}

/// Add the menu with the given name to the menu hierarchy (`add_menu_path`).
fn add_menu_path(
    menu_path: &CStr,
    menuarg: &mut Vimmenu,
    pri_tab: &[i32; MENUDEPTH + 1],
    call_data: Option<&CStr>,
) -> bool {
    let mut modes = menuarg.modes;
    let mut pri_idx: usize = 0;
    let mut old_modes: i32 = 0;

    let mut path_name_vec: Vec<u8> = menu_path.to_bytes_with_nul().to_vec();
    let mut ancestor_stack: Vec<usize> = Vec::new();
    let mut name_off = 0usize;

    let cleanup_erret = |ancestor_stack: &[usize]| -> bool {
        for depth in (0..ancestor_stack.len()).rev() {
            let mut mp_ref = &mut *g_st().root_menu.borrow_mut();
            for &idx in &ancestor_stack[..depth] {
                mp_ref = &mut mp_ref[idx].children;
            }
            let idx = ancestor_stack[depth];
            if mp_ref[idx].children.is_empty() {
                mp_ref.remove(idx);
            } else {
                break;
            }
        }
        false
    };

    while path_name_vec.get(name_off).is_some_and(|&b| b != 0) {
        let next_rel = menu_name_skip(&mut path_name_vec[name_off..]);
        let next_off = name_off + next_rel;
        let orig_name_cstr = crate::memory::bytes_as_cstr(&path_name_vec[name_off..]);
        let map_to = menutrans_lookup(orig_name_cstr.to_bytes());
        let (name_cstr, en_name_cstr) = if let Some(ref mapped) = map_to {
            (mapped.as_c_str(), Some(orig_name_cstr))
        } else {
            (orig_name_cstr, None)
        };
        let dname = menu_text(name_cstr.to_bytes(), None, None);
        if dname.as_bytes().is_empty() {
            emsg(gettext(c"E792: Empty menu name"));
            return cleanup_erret(&ancestor_stack);
        }

        let next_is_empty = path_name_vec.get(next_off).is_none_or(|&b| b == 0);
        let mut lower_pri_idx: usize = 0;
        let mut found_idx: Option<usize> = None;
        let mut root_menu_guard = g_st().root_menu.borrow_mut();
        let mut menup_vec = &mut *root_menu_guard;
        for &idx in &ancestor_stack {
            menup_vec = &mut menup_vec[idx].children;
        }
        for (idx, m) in menup_vec.iter_mut().enumerate() {
            if menu_name_equal(name_cstr.to_bytes(), m) || menu_name_equal(dname.as_bytes(), m) {
                if next_is_empty && !m.children.is_empty() {
                    if !g_st().sys_menu.get() {
                        emsg(gettext(c"E330: Menu path must not lead to a sub-menu"));
                    }
                    drop(root_menu_guard);
                    return cleanup_erret(&ancestor_stack);
                }
                if !next_is_empty && m.children.is_empty() {
                    if !g_st().sys_menu.get() {
                        emsg(gettext(E_NOTSUBMENU));
                    }
                    drop(root_menu_guard);
                    return cleanup_erret(&ancestor_stack);
                }
                found_idx = Some(idx);
                break;
            }

            if (!ancestor_stack.is_empty() || menu_is_menubar(m.name.as_bytes()))
                && m.priority <= pri_tab[pri_idx]
            {
                lower_pri_idx = idx + 1;
            }
        }

        let f_idx = if let Some(idx) = found_idx {
            let m = &mut menup_vec[idx];
            old_modes = m.modes;
            m.modes |= modes;
            m.enabled |= modes;
            idx
        } else {
            if next_is_empty && ancestor_stack.is_empty() {
                emsg(gettext(
                    c"E331: Must not add menu items directly to menu bar",
                ));
                drop(root_menu_guard);
                return cleanup_erret(&ancestor_stack);
            }

            if menu_is_separator(dname.as_bytes()) && !next_is_empty {
                emsg(gettext(c"E332: Separator cannot be part of a menu path"));
                drop(root_menu_guard);
                return cleanup_erret(&ancestor_stack);
            }

            let mut new_menu = Vimmenu::new();
            new_menu.modes = modes;
            new_menu.enabled = MENU_ALL_MODES;
            new_menu.name = name_cstr.to_owned();
            new_menu.dname = menu_text(
                name_cstr.to_bytes(),
                Some(&mut new_menu.mnemonic),
                Some(&mut new_menu.actext),
            );
            if let Some(en_name) = en_name_cstr {
                new_menu.en_name = Some(en_name.to_owned());
                new_menu.en_dname = Some(menu_text(en_name.to_bytes(), None, None));
            }
            new_menu.priority = pri_tab[pri_idx];

            menup_vec.insert(lower_pri_idx, new_menu);
            old_modes = 0;
            lower_pri_idx
        };

        ancestor_stack.push(f_idx);
        name_off = next_off;
        if pri_tab[pri_idx + 1] != -1 {
            pri_idx += 1;
        }
    }
    drop(path_name_vec);

    let amenu =
        (modes & (MENU_NORMAL_MODE | MENU_INSERT_MODE)) == (MENU_NORMAL_MODE | MENU_INSERT_MODE);
    if g_st().sys_menu.get() {
        modes &= !old_modes;
    }

    if !ancestor_stack.is_empty() && modes != 0 {
        let mut menup_vec = &mut *g_st().root_menu.borrow_mut();
        for &idx in &ancestor_stack[..ancestor_stack.len() - 1] {
            menup_vec = &mut menup_vec[idx].children;
        }
        let menu = &mut menup_vec[*ancestor_stack.last().unwrap()];
        let p = call_data.map(CStr::to_owned);
        let call_bytes = call_data.map_or(&[][..], CStr::to_bytes);

        for i in 0..MENU_MODES {
            if (modes & (1 << i)) != 0 {
                let mut c: u8 = 0;
                let mut d: u8 = 0;
                if amenu && !call_bytes.is_empty() {
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
                    let mut bytes = Vec::with_capacity(call_bytes.len() + 4);
                    bytes.push(c);
                    if d != 0 {
                        bytes.push(d);
                    }
                    bytes.extend_from_slice(call_bytes);
                    if c == CTRL_C_CONST {
                        bytes.push(CTRL_BSL_CONST);
                        bytes.push(CTRL_G);
                    }
                    menu.strings[i] = Some(std::ffi::CString::new(bytes).unwrap());
                } else {
                    menu.strings[i] = p.clone();
                }
                menu.noremap[i] = menuarg.noremap[0];
                menu.silent[i] = menuarg.silent[0];
            }
        }
    }
    true
}

/// Set the (sub)menu with the given name to enabled or disabled (`menu_enable_recurse`).
fn menu_enable_recurse(menus: &mut [Vimmenu], name: &CStr, modes: i32, enable: bool) -> bool {
    if menus.is_empty() {
        return true;
    }
    let mut name_buf = name.to_bytes_with_nul().to_vec();
    let p_off = menu_name_skip(&mut name_buf);
    let cur_name = crate::memory::bytes_as_cstr(&name_buf);
    let cur_bytes = cur_name.to_bytes();
    let p_cstr = crate::memory::bytes_as_cstr(&name_buf[p_off..]);

    let mut matched = false;
    for menu in menus.iter_mut() {
        if cur_bytes.is_empty()
            || cur_bytes.first() == Some(&b'*')
            || menu_name_equal(cur_bytes, menu)
        {
            matched = true;
            if !p_cstr.is_empty() {
                if menu.children.is_empty() {
                    emsg(gettext(E_NOTSUBMENU));
                    return false;
                }
                if !menu_enable_recurse(&mut menu.children, p_cstr, modes, enable) {
                    return false;
                }
            } else if enable {
                menu.enabled |= modes;
            } else {
                menu.enabled &= !modes;
            }

            if !cur_bytes.is_empty() && cur_bytes.first() != Some(&b'*') {
                break;
            }
        }
    }
    if !cur_bytes.is_empty() && cur_bytes.first() != Some(&b'*') && !matched {
        semsg!(gettext(E_NOMENU), cur_name);
        return false;
    }

    true
}

/// Remove the (sub)menu with the given name from the menu hierarchy (`remove_menu`).
fn remove_menu(menup: &mut Vec<Vimmenu>, name: &CStr, modes: i32, silent: bool) -> bool {
    if menup.is_empty() {
        return true;
    }
    let mut name_buf = name.to_bytes_with_nul().to_vec();
    let p_off = menu_name_skip(&mut name_buf);
    let cur_name = crate::memory::bytes_as_cstr(&name_buf);
    let cur_bytes = cur_name.to_bytes();
    let p_cstr = crate::memory::bytes_as_cstr(&name_buf[p_off..]);

    let mut idx: usize = 0;
    let mut found_menu = false;
    while idx < menup.len() {
        let menu = &mut menup[idx];
        if cur_bytes.is_empty() || menu_name_equal(cur_bytes, menu) {
            if !p_cstr.is_empty() && menu.children.is_empty() {
                if !silent {
                    emsg(gettext(E_NOTSUBMENU));
                }
                return false;
            }
            if (menu.modes & modes) != 0 {
                if !remove_menu(&mut menu.children, p_cstr, modes, silent) {
                    return false;
                }
            } else if !cur_bytes.is_empty() {
                if !silent {
                    emsg(gettext(E_MENU_ONLY_EXISTS_IN_ANOTHER_MODE));
                }
                return false;
            }

            if !cur_bytes.is_empty() {
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
    if !cur_bytes.is_empty() {
        if !found_menu {
            if !silent {
                semsg!(gettext(E_NOMENU), cur_name);
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

/// Export menus recursively (`menu_get_recursive`).
fn menu_get_recursive(menu: Option<&Vimmenu>, modes: i32) -> Option<crate::eval::typval::DictId> {
    let menu = menu?;
    if (menu.modes & modes) == 0 {
        return None;
    }

    let dict = tv_dict_alloc();
    tv_dict_add_str(&dict, c"name", Some(menu.dname.as_c_str()));
    tv_dict_add_nr(&dict, c"priority", i64::from(menu.priority));
    tv_dict_add_nr(
        &dict,
        c"hidden",
        i64::from(menu_is_hidden(menu.dname.as_bytes())),
    );

    if menu.mnemonic != 0 {
        let mut buf = [0u8; MB_MAXCHAR + 1];
        let buflen = utf_char2bytes(menu.mnemonic, &mut buf) as usize;
        tv_dict_add_str_len(&dict, c"shortcut", Some(&buf[..buflen]));
    }

    if let Some(ref actext) = menu.actext {
        tv_dict_add_str(&dict, c"actext", Some(actext.as_c_str()));
    }

    if (menu.modes & MENU_TIP_MODE) != 0
        && let Some(ref tip) = menu.strings[MENU_INDEX_TIP as usize]
    {
        tv_dict_add_str(&dict, c"tooltip", Some(tip.as_c_str()));
    }

    if menu.children.is_empty() {
        let commands = tv_dict_alloc();
        tv_dict_add_dict(&dict, c"mappings", Some(&commands));

        for (bit, mode_cstr) in MENU_MODE_CHARS.iter().enumerate() {
            if (menu.modes & modes & (1 << bit)) != 0 {
                let impl_dict = tv_dict_alloc();
                let s_bytes = menu.strings[bit].as_ref().map_or(&[][..], |s| s.as_bytes());
                let rhs = str2special_save(s_bytes, false, K_FALSE);
                tv_dict_add_str(&impl_dict, c"rhs", Some(rhs.as_c_str()));
                tv_dict_add_nr(&impl_dict, c"silent", i64::from(menu.silent[bit]));
                tv_dict_add_nr(
                    &impl_dict,
                    c"enabled",
                    if (menu.enabled & (1 << bit)) != 0 {
                        1
                    } else {
                        0
                    },
                );
                tv_dict_add_nr(
                    &impl_dict,
                    c"noremap",
                    if menu.noremap[bit] == REMAP_NONE {
                        1
                    } else {
                        0
                    },
                );
                tv_dict_add_nr(
                    &impl_dict,
                    c"sid",
                    if menu.noremap[bit] == REMAP_SCRIPT {
                        1
                    } else {
                        0
                    },
                );
                tv_dict_add_dict(&commands, mode_cstr, Some(&impl_dict));
            }
        }
    } else {
        let children_list = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
        for child in &menu.children {
            if let Some(d) = menu_get_recursive(Some(child), modes)
                && tv_dict_len(Some(&d)) > 0
            {
                tv_list_append_dict(&children_list, Some(&d));
            }
        }
        tv_dict_add_list(&dict, c"submenus", Some(&children_list));
    }
    Some(dict)
}

/// Export menus matching path `path_name` (`menu_get`).
pub fn menu_get(path_name: &CStr, modes: i32, list: &TvList) -> bool {
    let mut root = g_st().root_menu.borrow().clone();
    if !path_name.is_empty() {
        let Some(menu) = find_menu(&mut root, path_name, modes) else {
            return false;
        };
        if let Some(d) = menu_get_recursive(Some(menu), modes)
            && tv_dict_len(Some(&d)) > 0
        {
            tv_list_append_dict(list, Some(&d));
        }
        return true;
    }
    for menu in root.iter() {
        if let Some(d) = menu_get_recursive(Some(menu), modes)
            && tv_dict_len(Some(&d)) > 0
        {
            tv_list_append_dict(list, Some(&d));
        }
    }
    true
}

/// Find menu matching `path_name` and `modes` (`find_menu`).
fn find_menu<'a>(
    mut menus: &'a mut [Vimmenu],
    path_name: &CStr,
    modes: i32,
) -> Option<&'a mut Vimmenu> {
    debug_assert!(!path_name.is_empty());
    let mut saved_name_vec: Vec<u8> = path_name.to_bytes_with_nul().to_vec();
    let mut name_off = 0usize;

    while saved_name_vec.get(name_off).is_some_and(|&b| b != 0) {
        let p_off = name_off + menu_name_skip(&mut saved_name_vec[name_off..]);
        let cur_cstr = crate::memory::bytes_as_cstr(&saved_name_vec[name_off..]);
        let p_is_empty = saved_name_vec.get(p_off).is_none_or(|&b| b == 0);
        let mut found_idx: Option<usize> = None;
        for (idx, menu) in menus.iter_mut().enumerate() {
            if menu_name_equal(cur_cstr.to_bytes(), menu) {
                if !p_is_empty && menu.children.is_empty() {
                    emsg(gettext(E_NOTSUBMENU));
                    return None;
                } else if (menu.modes & modes) == 0 {
                    emsg(gettext(E_MENU_ONLY_EXISTS_IN_ANOTHER_MODE));
                    return None;
                }
                found_idx = Some(idx);
                break;
            }
        }

        let Some(idx) = found_idx else {
            semsg!(gettext(E_NOMENU), cur_cstr);
            return None;
        };
        if p_is_empty {
            return Some(&mut menus[idx]);
        }
        name_off = p_off;
        menus = &mut menus[idx].children;
    }

    None
}

/// Show the mapping associated with a menu item or hierarchy in a sub-menu (`show_menus`).
fn show_menus(path_name: &CStr, modes: i32) -> bool {
    let mut root = g_st().root_menu.borrow().clone();
    let mut menu: Option<&mut Vimmenu> = None;
    if !path_name.is_empty() {
        let Some(m) = find_menu(&mut root, path_name, modes) else {
            return false;
        };
        menu = Some(m);
    }

    MENU_STATE.with(|s| s.menus_locked.set(s.menus_locked.get() + (1)));
    msg_puts_title(gettext(c"\n--- Menus ---"));
    if menu.is_some() {
        show_menus_recursive(menu, modes, 0);
    } else {
        for child in &mut root {
            if g_st().got_int.get() {
                break;
            }
            if !menu_is_hidden(child.dname.as_bytes()) {
                show_menus_recursive(Some(child), modes, 0);
            }
        }
    }
    MENU_STATE.with(|s| s.menus_locked.set(s.menus_locked.get() - (1)));
    true
}

/// Recursively show the mappings associated with the menus under the given one (`show_menus_recursive`).
fn show_menus_recursive(mut menu: Option<&mut Vimmenu>, modes: i32, depth: i32) {
    if menu.is_some() && (menu.as_deref_mut().unwrap().modes & modes) == 0 {
        return;
    }

    if menu.is_some() {
        msg_putchar(b'\n' as i32);
        if g_st().got_int.get() {
            return;
        }
        for _ in 0..depth {
            msg_puts(c"  ");
        }
        if menu.as_deref_mut().unwrap().priority != 0 {
            msg_outnum(menu.as_deref_mut().unwrap().priority);
            msg_puts(c" ");
        }
        msg_outtrans(&menu.as_deref_mut().unwrap().name, HLF_D, false);
    }

    if menu.is_some() && menu.as_deref_mut().unwrap().children.is_empty() {
        for (bit, mode_cstr) in MENU_MODE_CHARS.iter().enumerate() {
            if (menu.as_deref_mut().unwrap().modes & modes & (1 << bit)) != 0 {
                msg_putchar(b'\n' as i32);
                if g_st().got_int.get() {
                    return;
                }
                for _ in 0..(depth + 2) {
                    msg_puts(c"  ");
                }
                msg_puts(mode_cstr);
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
                    msg_outtrans_special(s_bytes, false, 0);
                }
            }
        }
    } else if let Some(menu) = menu {
        for child in &mut menu.children {
            if g_st().got_int.get() {
                break;
            }
            if !menu_is_hidden(child.dname.as_bytes()) {
                show_menus_recursive(Some(&mut *child), modes, depth + 1);
            }
        }
    }
}

/// Work out what to complete when doing command line completion of menu names (`set_context_in_menu_cmd`).
pub fn set_context_in_menu_cmd(xp: &mut ExpandCtx, cmd: &CStr, arg_in: &CStr, forceit: bool) {
    let mut unmenu = false;

    xp.xp_context = EXPAND_UNSUCCESSFUL;

    let arg_bytes = arg_in.to_bytes();
    let mut p_idx = arg_bytes
        .iter()
        .take_while(|&&b| b.is_ascii_digit() || b == b'.')
        .count();

    if !arg_bytes
        .get(p_idx)
        .is_some_and(|&b| ascii_iswhite(b as i32))
    {
        if arg_bytes.starts_with(b"enable")
            && arg_bytes.get(6).is_none_or(|&b| ascii_iswhite(b as i32))
        {
            p_idx = 6;
        } else if arg_bytes.starts_with(b"disable")
            && arg_bytes.get(7).is_none_or(|&b| ascii_iswhite(b as i32))
        {
            p_idx = 7;
        } else {
            p_idx = 0;
        }
    }

    while p_idx < arg_bytes.len() && ascii_iswhite(arg_bytes[p_idx] as i32) {
        p_idx += 1;
    }

    let arg_start_idx = p_idx;
    let mut after_dot_idx = p_idx;

    while p_idx < arg_bytes.len() && !ascii_iswhite(arg_bytes[p_idx] as i32) {
        if (arg_bytes[p_idx] == b'\\' || arg_bytes[p_idx] == CTRL_V_CONST as u8)
            && p_idx + 1 < arg_bytes.len()
        {
            p_idx += 1;
        } else if arg_bytes[p_idx] == b'.' {
            after_dot_idx = p_idx + 1;
        }
        p_idx += 1;
    }

    let cmd_bytes = cmd.to_bytes();
    let expand_menus = !(cmd_bytes.starts_with(b"te") || cmd_bytes.first() == Some(&b'p'));
    MENU_STATE.with(|s| s.expand_emenu.set(cmd_bytes.first() == Some(&b'e')));
    if expand_menus
        && arg_bytes
            .get(p_idx)
            .is_some_and(|&b| ascii_iswhite(b as i32))
    {
        return;
    }
    if p_idx == arg_bytes.len() {
        MENU_STATE.with(|s| {
            s.expand_modes
                .set(get_menu_cmd_modes(cmd, forceit, None, Some(&mut unmenu)))
        });
        if !unmenu {
            MENU_STATE.with(|s| s.expand_modes.set(MENU_ALL_MODES));
        }

        let mut menu_list: &[Vimmenu] = &g_st().root_menu.borrow();
        let mut menu_path: Vec<usize> = Vec::new();
        if after_dot_idx > arg_start_idx {
            let path_len = after_dot_idx - arg_start_idx;
            let mut path_name_vec = vec![0; path_len];
            crate::memory::copy_bytes_to_c_buf(&mut path_name_vec, &arg_bytes[arg_start_idx..]);
            let mut name_off = 0usize;
            while path_name_vec.get(name_off).is_some_and(|&b| b != 0) {
                let next_off = name_off + menu_name_skip(&mut path_name_vec[name_off..]);
                let cur_bytes = crate::memory::bytes_until_nul(&path_name_vec[name_off..]);
                let next_non_empty = path_name_vec.get(next_off).is_some_and(|&b| b != 0);
                let mut found_idx: Option<usize> = None;
                for (idx, menu) in menu_list.iter().enumerate() {
                    if menu_name_equal(cur_bytes, menu) {
                        if (next_non_empty && menu.children.is_empty())
                            || (menu.modes & MENU_STATE.with(|s| s.expand_modes.get())) == 0
                        {
                            return;
                        }
                        found_idx = Some(idx);
                        break;
                    }
                }
                let Some(idx) = found_idx else {
                    return;
                };
                name_off = next_off;
                menu_path.push(idx);
                menu_list = &menu_list[idx].children;
            }
        }

        xp.xp_context = if expand_menus {
            EXPAND_MENUNAMES
        } else {
            EXPAND_MENUS
        };
        xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(arg_in).wrapping_add(after_dot_idx);
        EXPAND_MENU_PATH.with(|cell| {
            *cell.borrow_mut() = Some(menu_path);
        });
    } else {
        xp.xp_context = EXPAND_NOTHING;
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of (sub)menus (not entries) (`get_menu_name`).
pub fn get_menu_name(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    if idx == 0 {
        MENU_STATE.with(|s| s.get_menu_name_idx.set(0));
        MENU_STATE.with(|s| s.get_menu_name_advance.set(false));
    }

    let menus = expand_menu_slice()?;

    while MENU_STATE.with(|s| s.get_menu_name_idx.get()) < menus.len() {
        let menu = &menus[MENU_STATE.with(|s| s.get_menu_name_idx.get())];
        let dname_bytes = menu.dname.as_bytes();
        if menu_is_hidden(dname_bytes) || menu_is_separator(dname_bytes) || menu.children.is_empty()
        {
            MENU_STATE.with(|s| s.get_menu_name_idx.set(s.get_menu_name_idx.get() + (1)));
        } else {
            break;
        }
    }

    if MENU_STATE.with(|s| s.get_menu_name_idx.get()) >= menus.len() {
        return None;
    }
    let menu = &menus[MENU_STATE.with(|s| s.get_menu_name_idx.get())];

    let str_res = if (menu.modes & MENU_STATE.with(|s| s.expand_modes.get())) != 0 {
        if MENU_STATE.with(|s| s.get_menu_name_advance.get()) {
            menu.en_dname
                .as_deref()
                .map(crate::option_vars::intern_opt_cstr)
        } else {
            if menu.en_dname.is_none() {
                MENU_STATE.with(|s| s.get_menu_name_advance.set(true));
            }
            Some(crate::option_vars::intern_opt_cstr(menu.dname.as_c_str()))
        }
    } else {
        Some(c"")
    };

    if MENU_STATE.with(|s| s.get_menu_name_advance.get()) {
        MENU_STATE.with(|s| s.get_menu_name_idx.set(s.get_menu_name_idx.get() + (1)));
    }

    MENU_STATE.with(|s| {
        s.get_menu_name_advance
            .set(!MENU_STATE.with(|s| s.get_menu_name_advance.get()))
    });

    str_res
}

/// Function given to `ExpandGeneric()` to obtain the list of menus and menu entries (`get_menu_names`).
pub fn get_menu_names(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    if idx == 0 {
        MENU_STATE.with(|s| s.get_menu_names_idx.set(0));
        MENU_STATE.with(|s| s.get_menu_names_advance.set(false));
    }

    let menus = expand_menu_slice()?;

    while MENU_STATE.with(|s| s.get_menu_names_idx.get()) < menus.len() && {
        let dname_bytes = menus[MENU_STATE.with(|s| s.get_menu_names_idx.get())]
            .dname
            .as_bytes();
        menu_is_hidden(dname_bytes)
            || (MENU_STATE.with(|s| s.expand_emenu.get()) && menu_is_separator(dname_bytes))
            || dname_bytes.last() == Some(&b'.')
    } {
        MENU_STATE.with(|s| s.get_menu_names_idx.set(s.get_menu_names_idx.get() + (1)));
    }

    if MENU_STATE.with(|s| s.get_menu_names_idx.get()) >= menus.len() {
        return None;
    }
    let menu = &menus[MENU_STATE.with(|s| s.get_menu_names_idx.get())];

    let str_res = if (menu.modes & MENU_STATE.with(|s| s.expand_modes.get())) != 0 {
        if !menu.children.is_empty() {
            let mut tbuf = [0u8; TBUFFER_LEN];
            if MENU_STATE.with(|s| s.get_menu_names_advance.get()) {
                crate::memory::copy_bytes_to_c_buf(
                    &mut tbuf,
                    menu.en_dname.as_ref().map_or(&b""[..], |s| s.as_bytes()),
                );
            } else {
                crate::memory::copy_bytes_to_c_buf(&mut tbuf, menu.dname.as_bytes());
                if menu.en_dname.is_none() {
                    MENU_STATE.with(|s| s.get_menu_names_advance.set(true));
                }
            }
            let tlen = crate::memory::bytes_as_cstr(&tbuf).to_bytes().len();
            if tlen + 1 < TBUFFER_LEN {
                tbuf[tlen] = 1;
                tbuf[tlen + 1] = 0;
            }

            Some(xp.set_item(crate::memory::bytes_as_cstr(&tbuf).to_owned()))
        } else if MENU_STATE.with(|s| s.get_menu_names_advance.get()) {
            menu.en_dname
                .as_deref()
                .map(crate::option_vars::intern_opt_cstr)
        } else {
            if menu.en_dname.is_none() {
                MENU_STATE.with(|s| s.get_menu_names_advance.set(true));
            }
            Some(crate::option_vars::intern_opt_cstr(menu.dname.as_c_str()))
        }
    } else {
        Some(c"")
    };

    if MENU_STATE.with(|s| s.get_menu_names_advance.get()) {
        MENU_STATE.with(|s| s.get_menu_names_idx.set(s.get_menu_names_idx.get() + (1)));
    }

    MENU_STATE.with(|s| {
        s.get_menu_names_advance
            .set(!MENU_STATE.with(|s| s.get_menu_names_advance.get()))
    });

    str_res
}

/// Skip over this element of the menu path and return the start offset of the next element (`menu_name_skip`).
fn menu_name_skip(buf: &mut [u8]) -> usize {
    let mut len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let mut idx = 0;
    while idx < len && buf[idx] != b'.' {
        if buf[idx] == b'\\' || buf[idx] == CTRL_V_CONST as u8 {
            buf.copy_within(idx + 1..=len, idx);
            len -= 1;
            if idx >= len {
                break;
            }
        }
        idx += crate::mbyte::utfc_slice2len(&buf[idx..len]);
    }
    if idx < len {
        buf[idx] = 0;
        idx += 1;
    }
    idx
}

/// Return true when `name` matches with menu `menu` (`menu_name_equal`).
fn menu_name_equal(name: &[u8], menu: &Vimmenu) -> bool {
    if let Some(ref en_name) = menu.en_name
        && (menu_namecmp(name, en_name.as_bytes())
            || menu
                .en_dname
                .as_ref()
                .is_some_and(|ed| menu_namecmp(name, ed.as_bytes())))
    {
        return true;
    }
    menu_namecmp(name, menu.name.as_bytes()) || menu_namecmp(name, menu.dname.as_bytes())
}

fn menu_namecmp(name_bytes: &[u8], mname_bytes: &[u8]) -> bool {
    let n_end = name_bytes
        .iter()
        .position(|&b| b == TAB)
        .unwrap_or(name_bytes.len());
    let m_end = mname_bytes
        .iter()
        .position(|&b| b == TAB)
        .unwrap_or(mname_bytes.len());
    name_bytes[..n_end] == mname_bytes[..m_end]
}

/// Return the `MENU_MODES` specified by menu command `cmd` (`get_menu_cmd_modes`).
pub fn get_menu_cmd_modes(
    cmd: &CStr,
    forceit: bool,
    noremap: Option<&mut i32>,
    unmenu: Option<&mut bool>,
) -> i32 {
    let bytes = cmd.to_bytes();
    let first = bytes.first().copied().unwrap_or(0);
    let mut idx = 1usize;
    let next_b = bytes.get(idx).copied().unwrap_or(0);
    let modes = match first {
        b'v' => MENU_VISUAL_MODE | MENU_SELECT_MODE,
        b'x' => MENU_VISUAL_MODE,
        b's' => MENU_SELECT_MODE,
        b'o' => MENU_OP_PENDING_MODE,
        b'i' => MENU_INSERT_MODE,
        b't' => {
            if next_b == b'l' {
                idx += 1;
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
        b'n' if next_b != b'o' => MENU_NORMAL_MODE,
        _ => {
            idx = 0;
            if forceit {
                MENU_INSERT_MODE | MENU_CMDLINE_MODE
            } else {
                MENU_NORMAL_MODE | MENU_VISUAL_MODE | MENU_SELECT_MODE | MENU_OP_PENDING_MODE
            }
        }
    };

    let rem_first = bytes.get(idx).copied().unwrap_or(0);
    if let Some(noremap) = noremap {
        *noremap = if rem_first == b'n' {
            REMAP_NONE
        } else {
            REMAP_YES
        };
    }
    if let Some(unmenu) = unmenu {
        *unmenu = rem_first == b'u';
    }
    modes
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
fn popup_mode_name(name_bytes: &[u8], idx: usize) -> std::ffi::CString {
    debug_assert!(name_bytes.len() >= 4);

    let mode_chars = MENU_MODE_CHARS[idx].to_bytes();
    let mut out = Vec::with_capacity(name_bytes.len() + mode_chars.len());
    let split_at = name_bytes.len().min(5);
    out.extend_from_slice(&name_bytes[..split_at]);
    out.extend_from_slice(mode_chars);
    out.extend_from_slice(&name_bytes[split_at..]);
    std::ffi::CString::new(out).unwrap_or_default()
}

/// Duplicate the menu item text and then process mnemonic key and accelerator text (`menu_text`).
fn menu_text(
    full_bytes: &[u8],
    mut mnemonic: Option<&mut i32>,
    actext: Option<&mut Option<std::ffi::CString>>,
) -> std::ffi::CString {
    let src = if let Some(tab_pos) = full_bytes.iter().position(|&b| b == TAB) {
        if let Some(actext_out) = actext {
            *actext_out = Some(crate::memory::bytes_to_cstring(&full_bytes[tab_pos + 1..]));
        }
        &full_bytes[..tab_pos]
    } else {
        full_bytes
    };

    let mut out = Vec::with_capacity(src.len());
    let mut idx = 0usize;
    while idx < src.len() {
        if src[idx] == b'&' && idx + 1 < src.len() {
            if src[idx + 1] != b'&'
                && let Some(ref mut mnemonic) = mnemonic
            {
                **mnemonic = i32::from(src[idx + 1]);
            }
            out.push(src[idx + 1]);
            idx += 2;
        } else {
            out.push(src[idx]);
            idx += 1;
        }
    }
    crate::memory::bytes_to_cstring(&out)
}

/// Return true if `name` can be a menu in the MenuBar (`menu_is_menubar`).
pub fn menu_is_menubar(name: &[u8]) -> bool {
    !menu_is_popup(name)
        && !menu_is_toolbar(name)
        && !menu_is_winbar(name)
        && name.first().copied().unwrap_or(0) != MNU_HIDDEN_CHAR
}

/// Return true if `name` is a popup menu name (`menu_is_popup`).
pub fn menu_is_popup(name: &[u8]) -> bool {
    name.starts_with(b"PopUp")
}

/// Return true if `name` is a toolbar menu name (`menu_is_toolbar`).
pub fn menu_is_toolbar(name: &[u8]) -> bool {
    name.starts_with(b"ToolBar")
}

/// Return true if the name is a menu separator identifier (`menu_is_separator`).
pub fn menu_is_separator(name: &[u8]) -> bool {
    name.first() == Some(&b'-') && name.last() == Some(&b'-')
}

/// True if a popup menu or starts with `MNU_HIDDEN_CHAR` (`menu_is_hidden`).
fn menu_is_hidden(name: &[u8]) -> bool {
    name.first().copied().unwrap_or(0) == MNU_HIDDEN_CHAR || (menu_is_popup(name) && name.len() > 5)
}

fn get_menu_mode() -> i32 {
    if (g_st().state_global.get() & MODE_TERMINAL) != 0 {
        return MENU_INDEX_TERMINAL;
    }
    if g_st().visual_global.borrow().active {
        if g_st().visual_global.borrow().select {
            return MENU_INDEX_SELECT;
        }
        return MENU_INDEX_VISUAL;
    }
    if (g_st().state_global.get() & MODE_INSERT) != 0 {
        return MENU_INDEX_INSERT;
    }
    if (g_st().state_global.get() & MODE_CMDLINE) != 0
        || g_st().state_global.get() == MODE_ASKMORE
        || g_st().state_global.get() == MODE_HITRETURN
    {
        return MENU_INDEX_CMDLINE;
    }
    if g_st().finish_op.get() {
        return MENU_INDEX_OP_PENDING;
    }
    if (g_st().state_global.get() & MODE_NORMAL) != 0 {
        return MENU_INDEX_NORMAL;
    }
    if (g_st().state_global.get() & MODE_LANGMAP) != 0 {
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
    let menu_mode = get_menu_mode();
    if menu_mode == MENU_INDEX_INVALID {
        return;
    }
    let mode_cstr = MENU_MODE_CHARS[menu_mode as usize];

    apply_autocmds(
        EVENT_MENUPOPUP,
        Some(mode_cstr),
        None,
        false,
        curbuf_opt_mut().as_deref(),
    );

    let mut found_idx: Option<usize> = None;
    let mode_bytes = mode_cstr.to_bytes();
    for (idx, m) in g_st().root_menu.borrow().iter().enumerate() {
        if let Some(after) = m.name.as_bytes().strip_prefix(b"PopUp")
            && after.starts_with(mode_bytes)
        {
            found_idx = Some(idx);
            break;
        }
    }

    let Some(idx) = found_idx else {
        return;
    };
    if g_st().root_menu.borrow()[idx].children.is_empty() {
        return;
    }

    let mut m = g_st().root_menu.borrow()[idx].clone();
    pum_show_popupmenu(&mut m, None);
}

/// Execute `menu` (`execute_menu`).
pub fn execute_menu(eap: &Exarg, menu: &Vimmenu, mode_idx: i32) {
    let mut idx = mode_idx;

    if idx < 0 {
        if (g_st().state_global.get() & MODE_TERMINAL) != 0 {
            idx = MENU_INDEX_TERMINAL;
        } else if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
            idx = MENU_INDEX_CMDLINE;
        } else if (get_real_state() & MODE_VISUAL) != 0 {
            idx = MENU_INDEX_VISUAL;
        } else if ((g_st().state_global.get() & MODE_INSERT) != 0 || g_st().restart_edit.get() != 0)
            && g_st().current_sctx.get().sc_sid == 0
        {
            idx = MENU_INDEX_INSERT;
        } else if eap.addr_count != 0 {
            idx = MENU_INDEX_VISUAL;

            let tpos: CursorPos;
            if curbuf_mut().b_visual.borrow().vi_start.lnum == eap.line1
                && curbuf_mut().b_visual.borrow().vi_end.lnum == eap.line2
            {
                g_st().visual_global.borrow_mut().mode = curbuf_mut().b_visual.borrow().vi_mode;
                tpos = curbuf_mut().b_visual.borrow().vi_end;
                curwin_mut()
                    .w_cursor
                    .set(curbuf_mut().b_visual.borrow().vi_start);
                curwin_mut()
                    .w_curswant
                    .set(curbuf_mut().b_visual.borrow().vi_curswant);
            } else {
                g_st().visual_global.borrow_mut().mode = b'V' as i32;
                curwin_mut().w_cursor.set_lnum(eap.line1);
                curwin_mut().w_cursor.set_col(1);
                tpos = CursorPos {
                    lnum: eap.line2,
                    col: MAXCOL,
                    coladd: 0,
                };
            }

            g_st().visual_global.borrow_mut().active = true;
            g_st().visual_global.borrow_mut().reselect = 1;
            check_cursor(&curwin_mut());
            g_st().visual_global.borrow_mut().start = curwin_mut().w_cursor.get();
            curwin_mut().w_cursor.set(tpos);

            check_cursor(&curwin_mut());

            if opt_vars().p_sel.first_byte() == b'e' && gchar_cursor() != i32::from(NUL) {
                curwin_mut().w_cursor.add_col(1);
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
        if g_st().current_sctx.get().sc_sid != 0 {
            let mut save_state = SaveState::new();
            g_st().ex_normal_busy.set(g_st().ex_normal_busy.get() + (1));
            save_current_state(&mut save_state);
            exec_normal_cmd(cmd_str, menu.noremap[u_idx], menu.silent[u_idx]);
            restore_current_state(&mut save_state);
            g_st().ex_normal_busy.set(g_st().ex_normal_busy.get() - (1));
        } else {
            ins_typebuf(cmd_str, menu.noremap[u_idx], 0, true, menu.silent[u_idx]);
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
        semsg!(gettext(c"E335: Menu not defined for %s mode"), mode,);
    }
}

/// Lookup a menu by the descriptor name e.g. `"File.New"` (`menu_getbyname`).
fn menu_getbyname(name_arg: &CStr) -> Option<Vimmenu> {
    let mut saved_name_vec: Vec<u8> = name_arg.to_bytes_with_nul().to_vec();
    let root = g_st().root_menu.borrow().clone();
    let mut menus: &[Vimmenu] = root.as_slice();
    let mut name_off = 0usize;
    let mut gave_emsg = false;
    while saved_name_vec.get(name_off).is_some_and(|&b| b != 0) {
        let p_off = name_off + menu_name_skip(&mut saved_name_vec[name_off..]);
        let cur_bytes = crate::memory::bytes_until_nul(&saved_name_vec[name_off..]);
        let p_is_empty = saved_name_vec.get(p_off).is_none_or(|&b| b == 0);
        let mut found_idx: Option<usize> = None;

        for (idx, menu) in menus.iter().enumerate() {
            if menu_name_equal(cur_bytes, menu) {
                if p_is_empty && !menu.children.is_empty() {
                    emsg(gettext(c"E333: Menu path must lead to a menu item"));
                    gave_emsg = true;
                } else if !p_is_empty && menu.children.is_empty() {
                    emsg(gettext(E_NOTSUBMENU));
                } else {
                    found_idx = Some(idx);
                }
                break;
            }
        }
        let Some(idx) = found_idx else {
            break;
        };
        if p_is_empty {
            return Some(menus[idx].clone());
        }
        menus = menus[idx].children.as_slice();
        name_off = p_off;
    }
    drop(saved_name_vec);
    if !gave_emsg {
        semsg!(gettext(c"E334: Menu not found: %s"), name_arg);
    }
    None
}

/// `:emenu` (`ex_emenu`).
pub fn ex_emenu(eap: &mut Exarg) {
    let mut arg = eap.arg.clone();
    let mut mode_idx = MENU_INDEX_INVALID;

    let arg_bytes = (arg).as_bytes();
    if let Some(&b0) = arg_bytes.first()
        && ascii_iswhite(i32::from(arg_bytes.get(1).copied().unwrap_or(0)))
    {
        match b0 {
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
        arg = arg.wrapping_add(2).skipwhite();
    }

    let Some(menu) = menu_getbyname((arg).as_cstr()) else {
        return;
    };

    execute_menu(eap, &menu, mode_idx);
}

/// Given a menu descriptor, e.g. `"File.New"`, find it in the menu hierarchy (`menu_find`).
pub fn menu_find(path_name: &CStr) -> Option<Vimmenu> {
    let root = g_st().root_menu.borrow().clone();
    let mut menus: &[Vimmenu] = root.as_slice();
    let mut saved_name_vec: Vec<u8> = path_name.to_bytes_with_nul().to_vec();
    let mut name_off = 0usize;
    while saved_name_vec.get(name_off).is_some_and(|&b| b != 0) {
        let p_off = name_off + menu_name_skip(&mut saved_name_vec[name_off..]);
        let cur_bytes = crate::memory::bytes_until_nul(&saved_name_vec[name_off..]);
        let p_is_empty = saved_name_vec.get(p_off).is_none_or(|&b| b == 0);
        let mut found_idx: Option<usize> = None;

        for (idx, menu) in menus.iter().enumerate() {
            if menu_name_equal(cur_bytes, menu) {
                if menu.children.is_empty() {
                    if p_is_empty {
                        emsg(gettext(c"E336: Menu path must lead to a sub-menu"));
                    } else {
                        emsg(gettext(E_NOTSUBMENU));
                    }
                    return None;
                }
                found_idx = Some(idx);
                break;
            }
        }
        let Some(idx) = found_idx else {
            break;
        };
        if p_is_empty {
            return Some(menus[idx].clone());
        }

        menus = menus[idx].children.as_slice();
        name_off = p_off;
    }

    emsg(gettext(c"E337: Menu not found - check menu names"));
    None
}

/// `:menutranslate` (`ex_menutranslate`).
pub fn ex_menutranslate(eap: &mut Exarg) {
    if eap.arg.as_bytes().starts_with(b"clear")
        && ends_excmd(i32::from(eap.arg.wrapping_add(5).skipwhite().first_byte())) != 0
    {
        MENUTRANS_GA.with(|cell| cell.borrow_mut().clear());
        del_menutrans_vars();
    } else {
        let arg_bytes = eap.arg.as_bytes();
        let part1_len = menu_skip_part(arg_bytes);
        let to_off = part1_len + crate::charset::skipwhite_len(&arg_bytes[part1_len..]);
        let part2_len = menu_skip_part(&arg_bytes[to_off..]);
        if part2_len == 0 {
            emsg(gettext(E_INVARG));
        } else {
            let mut from_vec: Vec<u8> = Vec::with_capacity(part1_len + 1);
            from_vec.extend_from_slice(&arg_bytes[..part1_len]);
            from_vec.push(0);
            let from_noamp_cstr = menu_text(&from_vec[..part1_len], None, None);
            let mut to_vec: Vec<u8> = Vec::with_capacity(part2_len + 1);
            to_vec.extend_from_slice(&arg_bytes[to_off..to_off + part2_len]);
            to_vec.push(0);
            menu_translate_tab_and_shift(&mut from_vec);
            menu_translate_tab_and_shift(&mut to_vec);
            menu_unescape_name(&mut from_vec);
            menu_unescape_name(&mut to_vec);
            let from_cstr = crate::memory::bytes_as_cstr(&from_vec).to_owned();
            let to_cstr = crate::memory::bytes_as_cstr(&to_vec).to_owned();
            MENUTRANS_GA.with(|cell| {
                cell.borrow_mut().push(Menutrans {
                    from: from_cstr,
                    from_noamp: from_noamp_cstr,
                    to: to_cstr,
                });
            });
        }
    }
}

/// Find the offset just after one part of a menu name (`menu_skip_part`).
fn menu_skip_part(p: &[u8]) -> usize {
    let mut idx = 0usize;
    while idx < p.len() && p[idx] != 0 && p[idx] != b'.' && !ascii_iswhite(i32::from(p[idx])) {
        if (p[idx] == b'\\' || p[idx] == CTRL_V_CONST as u8) && idx + 1 < p.len() && p[idx + 1] != 0
        {
            idx += 1;
        }
        idx += 1;
    }
    idx
}

/// Lookup part of a menu name in the translations (`menutrans_lookup`).
fn menutrans_lookup(name_slice: &[u8]) -> Option<std::ffi::CString> {
    let u_len = name_slice.len();
    let dname = menu_text(name_slice, None, None);

    MENUTRANS_GA.with(|cell| {
        let menutrans = cell.borrow();
        for entry in menutrans.iter() {
            let from_bytes = entry.from.as_bytes();
            if from_bytes.len() == u_len && name_slice.eq_ignore_ascii_case(from_bytes) {
                return Some(entry.to.clone());
            }
        }

        for entry in menutrans.iter() {
            if dname
                .as_bytes()
                .eq_ignore_ascii_case(entry.from_noamp.as_bytes())
            {
                return Some(entry.to.clone());
            }
        }

        None
    })
}

/// Unescape the name in the translate dictionary table (`menu_unescape_name`).
fn menu_unescape_name(buf: &mut [u8]) {
    let mut len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let mut idx = 0;
    while idx < len && buf[idx] != b'.' {
        if buf[idx] == b'\\' {
            buf.copy_within(idx + 1..=len, idx);
            len -= 1;
        }
        if idx < len {
            idx += crate::mbyte::utfc_slice2len(&buf[idx..len]);
        }
    }
}

/// Isolate the menu name (`menu_translate_tab_and_shift`).
fn menu_translate_tab_and_shift(buf: &mut [u8]) -> usize {
    let mut len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let mut idx = 0;
    while idx < len && !ascii_iswhite(i32::from(buf[idx])) {
        if (buf[idx] == b'\\' || buf[idx] == CTRL_V_CONST as u8) && idx + 1 < len {
            idx += 1;
        } else if len - idx >= 5 && buf[idx..idx + 5].eq_ignore_ascii_case(b"<TAB>") {
            buf[idx] = b'\t';
            buf.copy_within(idx + 5..=len, idx + 1);
            len -= 4;
        }
        idx += 1;
    }
    if idx < len {
        buf[idx] = 0;
        idx += 1;
    }
    let rem = crate::charset::skipwhite_slice(&buf[idx..len]);
    len - rem.len()
}

/// Get the information about a menu item in mode `modes` (`menuitem_getinfo`).
fn menuitem_getinfo(menu: &Vimmenu, modes: i32, dict: &TvDict) {
    tv_dict_add_str(dict, c"name", Some(menu.name.as_c_str()));
    tv_dict_add_str(dict, c"display", Some(menu.dname.as_c_str()));
    if let Some(ref actext) = menu.actext {
        tv_dict_add_str(dict, c"accel", Some(actext.as_c_str()));
    }
    tv_dict_add_nr(dict, c"priority", i64::from(menu.priority));
    tv_dict_add_str(dict, c"modes", Some(get_menu_mode_str(menu.modes)));

    let mut buf = [0u8; NUMBUFLEN];
    let buflen = utf_char2bytes(menu.mnemonic, &mut buf) as usize;
    buf[buflen] = 0;
    tv_dict_add_str_len(dict, c"shortcut", Some(&buf[..buflen]));

    if menu.children.is_empty() {
        let mut bit = 0usize;
        while bit < MENU_MODES && ((1 << bit) & modes) == 0 {
            bit += 1;
        }

        if bit < MENU_MODES {
            if let Some(ref s) = menu.strings[bit] {
                if s.as_bytes().is_empty() {
                    tv_dict_add_str(dict, c"rhs", Some(c"<Nop>"));
                } else {
                    let rhs = str2special_save(s.as_bytes(), false, K_FALSE);
                    tv_dict_add_str(dict, c"rhs", Some(rhs.as_c_str()));
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
        tv_dict_add_list(dict, c"submenus", Some(&l));
        for child in &menu.children {
            tv_list_append_string(&l, Some(child.dname.as_c_str()), -1);
        }
    }
}

/// `menu_info()` function (`f_menu_info`).
pub fn f_menu_info(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let retdict = tv_dict_alloc_ret(rettv);

    let Some(menu_name) = tv_get_string_chk(&argvars[0]) else {
        return;
    };

    let which_cow;
    let which: &CStr = if argvars[1].v_type != VarType::VarUnknown {
        let Some(w) = tv_get_string_chk(&argvars[1]) else {
            return;
        };
        which_cow = w;
        &which_cow
    } else {
        c""
    };

    let modes = get_menu_cmd_modes(which, which.to_bytes().first() == Some(&b'!'), None, None);

    let root = g_st().root_menu.borrow().clone();
    let mut menus: &[Vimmenu] = root.as_slice();
    if menu_name.is_empty() {
        if !menus.is_empty() && (menus[0].modes & modes) != 0 {
            let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
            tv_dict_add_list(&retdict, c"submenus", Some(&l));
            for topmenu in menus {
                if !menu_is_hidden(topmenu.dname.as_bytes()) {
                    tv_list_append_string(&l, Some(topmenu.dname.as_c_str()), -1);
                }
            }
        }
        return;
    }

    let mut saved_name_vec: Vec<u8> = menu_name.to_bytes_with_nul().to_vec();
    let mut found: Option<&Vimmenu> = None;
    let mut name_off = 0usize;
    while saved_name_vec.get(name_off).is_some_and(|&b| b != 0) {
        let p_off = name_off + menu_name_skip(&mut saved_name_vec[name_off..]);
        let cur_bytes = crate::memory::bytes_until_nul(&saved_name_vec[name_off..]);
        found = None;
        for menu in menus {
            if menu_name_equal(cur_bytes, menu) {
                found = Some(menu);
                break;
            }
        }
        let Some(f) = found else {
            break;
        };
        if saved_name_vec.get(p_off).is_none_or(|&b| b == 0) {
            break;
        }
        menus = f.children.as_slice();
        name_off = p_off;
    }
    drop(saved_name_vec);

    if let Some(found) = found
        && (found.modes & modes) != 0
    {
        menuitem_getinfo(found, modes, &retdict);
    }
}
