//! Code for menus, used for the GUI and `'wildmenu'` (`src/nvim/menu.c`).

use crate::autocmd::{EVENT_MENUPOPUP, HLF_8, apply_autocmds};
use crate::charset::{Ctrl_V, ascii_isdigit, ascii_iswhite, getdigits_int, skipwhite};
use crate::cursor::{check_cursor, gchar_cursor, pos_T};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, VarType, dict_T, list_T, tv_dict_add_allocated_str, tv_dict_add_bool,
    tv_dict_add_dict, tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_add_str_len,
    tv_dict_alloc, tv_dict_alloc_ret, tv_get_string_chk, tv_list_alloc, tv_list_append_dict,
    tv_list_append_string, typval_T,
};
use crate::eval::vars::del_menutrans_vars;
use crate::ex_docmd::{
    Ctrl_C, Ctrl_G, Ctrl_O, ends_excmd, exarg_T, exec_normal_cmd, restore_current_state,
    save_current_state,
};
use crate::garray::{ga_clear, ga_grow, ga_init, garray_T};
use crate::globals::*;
use crate::input::{REMAP_SCRIPT, ins_typebuf, tasave_T};
use crate::keycodes::{REPTERM_DO_LT, replace_termcodes};
use crate::mbyte::{utf_char2bytes, utfc_ptr2len};
use crate::memory::{xcalloc, xfree, xmalloc, xmemdupz, xstrdup, xstrlcpy};
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
    gettext, strcat, strcmp, strcpy, strmove, strncmp, vim_strchr, vim_stricmp, vim_strnicmp_asc,
    xstrnsave,
};
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
    pub name: *mut c_char,
    pub dname: *mut c_char,
    pub en_name: *mut c_char,
    pub en_dname: *mut c_char,
    pub mnemonic: c_int,
    pub actext: *mut c_char,
    pub priority: c_int,
    pub strings: [*mut c_char; MENU_MODES],
    pub noremap: [c_int; MENU_MODES],
    pub silent: [bool; MENU_MODES],
    pub children: *mut vimmenu_T,
    pub parent: *mut vimmenu_T,
    pub next: *mut vimmenu_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct menutrans_T {
    from: *mut c_char,
    from_noamp: *mut c_char,
    to: *mut c_char,
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
#[derive(Clone, Copy)]
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

static mut MENUS_LOCKED: c_int = 0;
static mut EXPAND_MENU: *mut vimmenu_T = ptr::null_mut();
static mut EXPAND_MODES: c_int = 0;
static mut EXPAND_EMENU: bool = false;
static mut GET_MENU_NAME_MENU: *mut vimmenu_T = ptr::null_mut();
static mut GET_MENU_NAME_ADVANCE: bool = false;
static mut GET_MENU_NAMES_MENU: *mut vimmenu_T = ptr::null_mut();
static mut GET_MENU_NAMES_ADVANCE: bool = false;
static mut GET_MENU_NAMES_TBUFFER: [c_char; TBUFFER_LEN] = [0; TBUFFER_LEN];
static mut MENUTRANS_GA: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: 0,
    ga_growsize: 0,
    ga_data: ptr::null_mut(),
};

use crate::eval::typval::tv_dict_len;

/// Return true if `name` is a window toolbar menu name (`menu_is_winbar`).
unsafe fn menu_is_winbar(name: *const c_char) -> bool {
    unsafe { strncmp(name, c"WinBar".as_ptr(), 6) == 0 }
}

unsafe fn get_root_menu(_name: *const c_char) -> *mut *mut vimmenu_T {
    &raw mut root_menu
}

/// If `menus_locked` is set then give an error and return true (`is_menus_locked`).
unsafe fn is_menus_locked() -> bool {
    unsafe {
        if MENUS_LOCKED > 0 {
            emsg(gettext(e_cannot_change_menus_while_listing.as_ptr()));
            return true;
        }
        false
    }
}

/// Do the `:menu` command and relatives (`ex_menu`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_menu(eap: *mut exarg_T) {
    unsafe {
        let mut noremap: c_int = 0;
        let mut silent = false;
        let mut unmenu = false;
        let mut pri_tab = [0 as c_int; MENUDEPTH + 1];
        let mut enable: c_int = kNone;

        let modes = get_menu_cmd_modes(
            (*eap).cmd,
            (*eap).forceit != 0,
            &raw mut noremap,
            &raw mut unmenu,
        );
        let mut arg = (*eap).arg;

        loop {
            if strncmp(arg, c"<script>".as_ptr(), 8) == 0 {
                noremap = REMAP_SCRIPT;
                arg = skipwhite(arg.add(8));
                continue;
            }
            if strncmp(arg, c"<silent>".as_ptr(), 8) == 0 {
                silent = true;
                arg = skipwhite(arg.add(8));
                continue;
            }
            if strncmp(arg, c"<special>".as_ptr(), 9) == 0 {
                arg = skipwhite(arg.add(9));
                continue;
            }
            break;
        }

        if strncmp(arg, c"icon=".as_ptr(), 5) == 0 {
            arg = arg.add(5);
            while *arg != NUL && *arg != b' ' as c_char {
                if *arg == b'\\' as c_char {
                    strmove(arg, arg.add(1));
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
        } else if (*eap).addr_count != 0 && (*eap).line2 != 0 {
            pri_tab[0] = (*eap).line2;
            i = 1;
        } else {
            i = 0;
        }
        while i < MENUDEPTH {
            pri_tab[i] = 500;
            i += 1;
        }
        pri_tab[MENUDEPTH] = -1;

        if strncmp(arg, c"enable".as_ptr(), 6) == 0 && ascii_iswhite(*arg.add(6)) {
            enable = kTrue;
            arg = skipwhite(arg.add(6));
        } else if strncmp(arg, c"disable".as_ptr(), 7) == 0 && ascii_iswhite(*arg.add(7)) {
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
            if strcmp(menu_path, c"*".as_ptr()) == 0 {
                menu_path = c"".as_ptr().cast_mut();
            }

            if menu_is_popup(menu_path) {
                for idx in 0..MENU_INDEX_TIP {
                    if (modes & (1 << idx)) != 0 {
                        p = popup_mode_name(menu_path, idx as usize);
                        menu_enable_recurse(*root_menu_ptr, p, MENU_ALL_MODES, enable != 0);
                        xfree(p.cast());
                    }
                }
            }
            menu_enable_recurse(*root_menu_ptr, menu_path, modes, enable != 0);
        } else if unmenu {
            if is_menus_locked() {
                return;
            }

            if strcmp(menu_path, c"*".as_ptr()) == 0 {
                menu_path = c"".as_ptr().cast_mut();
            }

            if menu_is_popup(menu_path) {
                for idx in 0..MENU_INDEX_TIP {
                    if (modes & (1 << idx)) != 0 {
                        p = popup_mode_name(menu_path, idx as usize);
                        remove_menu(root_menu_ptr, p, MENU_ALL_MODES, true);
                        xfree(p.cast());
                    }
                }
            }

            remove_menu(root_menu_ptr, menu_path, modes, false);
        } else {
            if is_menus_locked() {
                return;
            }

            let mut map_buf: *mut c_char = ptr::null_mut();
            if vim_stricmp(map_to, c"<nop>".as_ptr()) == 0 {
                map_to = c"".as_ptr().cast_mut();
            } else if (modes & MENU_TIP_MODE) == 0 {
                map_to = replace_termcodes(
                    map_to,
                    CStr::from_ptr(map_to).to_bytes().len(),
                    &raw mut map_buf,
                    0,
                    REPTERM_DO_LT,
                    ptr::null_mut(),
                    p_cpo,
                );
            }

            let mut menuarg: vimmenu_T = std::mem::zeroed();
            menuarg.modes = modes;
            menuarg.noremap[0] = noremap;
            menuarg.silent[0] = silent;
            add_menu_path(menu_path, &raw mut menuarg, pri_tab.as_ptr(), map_to);

            if menu_is_popup(menu_path) {
                for idx in 0..MENU_INDEX_TIP {
                    if (modes & (1 << idx)) != 0 {
                        p = popup_mode_name(menu_path, idx as usize);
                        menuarg.modes = modes;
                        add_menu_path(p, &raw mut menuarg, pri_tab.as_ptr(), map_to);
                        xfree(p.cast());
                    }
                }
            }

            xfree(map_buf.cast());
        }

        ui_call_update_menu();
    }
}

/// Add the menu with the given name to the menu hierarchy (`add_menu_path`).
unsafe fn add_menu_path(
    menu_path: *const c_char,
    menuarg: *mut vimmenu_T,
    pri_tab: *const c_int,
    call_data: *const c_char,
) -> c_int {
    unsafe {
        let mut modes = (*menuarg).modes;
        let mut menu: *mut vimmenu_T = ptr::null_mut();
        let mut dname: *mut c_char = ptr::null_mut();
        let mut pri_idx: usize = 0;
        let mut old_modes: c_int = 0;

        let path_name = xstrdup(menu_path);
        let root_menu_ptr = get_root_menu(menu_path);
        let mut menup: *mut *mut vimmenu_T = root_menu_ptr;
        let mut parent: *mut vimmenu_T = ptr::null_mut();
        let mut name = path_name;

        let cleanup_erret = |path_name: *mut c_char,
                             dname: *mut c_char,
                             mut parent: *mut vimmenu_T,
                             root_menu_ptr: *mut *mut vimmenu_T|
         -> c_int {
            xfree(path_name.cast());
            xfree(dname.cast());

            while !parent.is_null() && (*parent).children.is_null() {
                let mut mp = if (*parent).parent.is_null() {
                    root_menu_ptr
                } else {
                    &raw mut (*(*parent).parent).children
                };
                while !(*mp).is_null() && *mp != parent {
                    mp = &raw mut (*(*mp)).next;
                }
                if (*mp).is_null() {
                    break;
                }
                parent = (*parent).parent;
                free_menu(mp);
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
            dname = menu_text(name, ptr::null_mut(), ptr::null_mut());
            if *dname == NUL {
                emsg(gettext(c"E792: Empty menu name".as_ptr()));
                return cleanup_erret(path_name, dname, parent, root_menu_ptr);
            }

            let mut lower_pri = menup;
            menu = *menup;
            while !menu.is_null() {
                if menu_name_equal(name, menu) || menu_name_equal(dname, menu) {
                    if *next_name == NUL && !(*menu).children.is_null() {
                        if !sys_menu {
                            emsg(gettext(
                                c"E330: Menu path must not lead to a sub-menu".as_ptr(),
                            ));
                        }
                        return cleanup_erret(path_name, dname, parent, root_menu_ptr);
                    }
                    if *next_name != NUL && (*menu).children.is_null() {
                        if !sys_menu {
                            emsg(gettext(E_NOTSUBMENU.as_ptr()));
                        }
                        return cleanup_erret(path_name, dname, parent, root_menu_ptr);
                    }
                    break;
                }
                menup = &raw mut (*menu).next;

                if (!parent.is_null() || menu_is_menubar((*menu).name))
                    && (*menu).priority <= *pri_tab.add(pri_idx)
                {
                    lower_pri = menup;
                }
                menu = (*menu).next;
            }

            if menu.is_null() {
                if *next_name == NUL && parent.is_null() {
                    emsg(gettext(
                        c"E331: Must not add menu items directly to menu bar".as_ptr(),
                    ));
                    return cleanup_erret(path_name, dname, parent, root_menu_ptr);
                }

                if menu_is_separator(dname) && *next_name != NUL {
                    emsg(gettext(
                        c"E332: Separator cannot be part of a menu path".as_ptr(),
                    ));
                    return cleanup_erret(path_name, dname, parent, root_menu_ptr);
                }

                menu = xcalloc(1, size_of::<vimmenu_T>()).cast::<vimmenu_T>();
                (*menu).modes = modes;
                (*menu).enabled = MENU_ALL_MODES;
                (*menu).name = xstrdup(name);
                (*menu).dname = menu_text(name, &raw mut (*menu).mnemonic, &raw mut (*menu).actext);
                if !en_name.is_null() {
                    (*menu).en_name = xstrdup(en_name);
                    (*menu).en_dname = menu_text(en_name, ptr::null_mut(), ptr::null_mut());
                } else {
                    (*menu).en_name = ptr::null_mut();
                    (*menu).en_dname = ptr::null_mut();
                }
                (*menu).priority = *pri_tab.add(pri_idx);
                (*menu).parent = parent;

                (*menu).next = *lower_pri;
                *lower_pri = menu;

                old_modes = 0;
            } else {
                old_modes = (*menu).modes;
                (*menu).modes |= modes;
                (*menu).enabled |= modes;
            }

            menup = &raw mut (*menu).children;
            parent = menu;
            name = next_name;
            xfree(dname.cast());
            dname = ptr::null_mut();
            if *pri_tab.add(pri_idx + 1) != -1 {
                pri_idx += 1;
            }
        }
        xfree(path_name.cast());
        let _ = dname;

        let amenu = (modes & (MENU_NORMAL_MODE | MENU_INSERT_MODE))
            == (MENU_NORMAL_MODE | MENU_INSERT_MODE);
        if sys_menu {
            modes &= !old_modes;
        }

        if !menu.is_null() && modes != 0 {
            let p = if call_data.is_null() {
                ptr::null_mut()
            } else {
                xstrdup(call_data)
            };

            for i in 0..MENU_MODES {
                if (modes & (1 << i)) != 0 {
                    free_menu_string(menu, i);

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
                        let s_buf = xmalloc(CStr::from_ptr(call_data).to_bytes().len() + 5)
                            .cast::<c_char>();
                        (*menu).strings[i] = s_buf;
                        *s_buf = c;
                        if d == 0 {
                            strcpy(s_buf.add(1), call_data);
                        } else {
                            *s_buf.add(1) = d;
                            strcpy(s_buf.add(2), call_data);
                        }
                        if c == Ctrl_C {
                            let len = CStr::from_ptr(s_buf).to_bytes().len();
                            *s_buf.add(len) = Ctrl_BSL;
                            *s_buf.add(len + 1) = Ctrl_G;
                            *s_buf.add(len + 2) = NUL;
                        }
                    } else {
                        (*menu).strings[i] = p;
                    }
                    (*menu).noremap[i] = (*menuarg).noremap[0];
                    (*menu).silent[i] = (*menuarg).silent[0];
                }
            }
        }
        OK
    }
}

/// Set the (sub)menu with the given name to enabled or disabled (`menu_enable_recurse`).
unsafe fn menu_enable_recurse(
    mut menu: *mut vimmenu_T,
    name: *mut c_char,
    modes: c_int,
    enable: bool,
) -> c_int {
    unsafe {
        if menu.is_null() {
            return OK;
        }
        let p = menu_name_skip(name);

        while !menu.is_null() {
            if *name == NUL || *name == b'*' as c_char || menu_name_equal(name, menu) {
                if *p != NUL {
                    if (*menu).children.is_null() {
                        emsg(gettext(E_NOTSUBMENU.as_ptr()));
                        return FAIL;
                    }
                    if menu_enable_recurse((*menu).children, p, modes, enable) == FAIL {
                        return FAIL;
                    }
                } else if enable {
                    (*menu).enabled |= modes;
                } else {
                    (*menu).enabled &= !modes;
                }

                if *name != NUL && *name != b'*' as c_char {
                    break;
                }
            }
            menu = (*menu).next;
        }
        if *name != NUL && *name != b'*' as c_char && menu.is_null() {
            semsg!(gettext(E_NOMENU.as_ptr()), name);
            return FAIL;
        }

        OK
    }
}

/// Remove the (sub)menu with the given name from the menu hierarchy (`remove_menu`).
unsafe fn remove_menu(
    mut menup: *mut *mut vimmenu_T,
    name: *mut c_char,
    modes: c_int,
    silent: bool,
) -> c_int {
    unsafe {
        if (*menup).is_null() {
            return OK;
        }
        let p = menu_name_skip(name);

        let mut menu: *mut vimmenu_T;
        loop {
            menu = *menup;
            if menu.is_null() {
                break;
            }
            if *name == NUL || menu_name_equal(name, menu) {
                if *p != NUL && (*menu).children.is_null() {
                    if !silent {
                        emsg(gettext(E_NOTSUBMENU.as_ptr()));
                    }
                    return FAIL;
                }
                if ((*menu).modes & modes) != 0 {
                    if remove_menu(&raw mut (*menu).children, p, modes, silent) == FAIL {
                        return FAIL;
                    }
                } else if *name != NUL {
                    if !silent {
                        emsg(gettext(e_menu_only_exists_in_another_mode.as_ptr()));
                    }
                    return FAIL;
                }

                if *name != NUL {
                    break;
                }

                (*menu).modes &= !modes;
                if (modes & MENU_TIP_MODE) != 0 {
                    free_menu_string(menu, MENU_INDEX_TIP as usize);
                }
                if ((*menu).modes & MENU_ALL_MODES) == 0 {
                    free_menu(menup);
                } else {
                    menup = &raw mut (*menu).next;
                }
            } else {
                menup = &raw mut (*menu).next;
            }
        }
        if *name != NUL {
            if menu.is_null() {
                if !silent {
                    semsg!(gettext(E_NOMENU.as_ptr()), name);
                }
                return FAIL;
            }

            (*menu).modes &= !modes;
            let mut child = (*menu).children;
            while !child.is_null() {
                (*menu).modes |= (*child).modes;
                child = (*child).next;
            }
            if (modes & MENU_TIP_MODE) != 0 {
                free_menu_string(menu, MENU_INDEX_TIP as usize);
            }
            if ((*menu).modes & MENU_ALL_MODES) == 0 {
                *menup = menu;
                free_menu(menup);
            }
        }

        OK
    }
}

/// Free the given menu structure and remove it from the linked list (`free_menu`).
unsafe fn free_menu(menup: *mut *mut vimmenu_T) {
    unsafe {
        let menu = *menup;
        *menup = (*menu).next;
        xfree((*menu).name.cast());
        xfree((*menu).dname.cast());
        xfree((*menu).en_name.cast());
        xfree((*menu).en_dname.cast());
        xfree((*menu).actext.cast());
        for i in 0..MENU_MODES {
            free_menu_string(menu, i);
        }
        xfree(menu.cast());
    }
}

/// Free the `menu->strings[idx]` with the given index (`free_menu_string`).
unsafe fn free_menu_string(menu: *mut vimmenu_T, idx: usize) {
    unsafe {
        let mut count: c_int = 0;
        for i in 0..MENU_MODES {
            if (*menu).strings[i] == (*menu).strings[idx] {
                count += 1;
            }
        }
        if count == 1 {
            xfree((*menu).strings[idx].cast());
        }
        (*menu).strings[idx] = ptr::null_mut();
    }
}

/// Export menus recursively (`menu_get_recursive`).
unsafe fn menu_get_recursive(mut menu: *const vimmenu_T, modes: c_int) -> *mut dict_T {
    unsafe {
        if menu.is_null() || ((*menu).modes & modes) == 0 {
            return ptr::null_mut();
        }

        let dict = tv_dict_alloc();
        tv_dict_add_str(dict, c"name".as_ptr(), 4, (*menu).dname);
        tv_dict_add_nr(dict, c"priority".as_ptr(), 8, i64::from((*menu).priority));
        tv_dict_add_nr(
            dict,
            c"hidden".as_ptr(),
            6,
            i64::from(menu_is_hidden((*menu).dname)),
        );

        if (*menu).mnemonic != 0 {
            let mut buf = [0 as c_char; MB_MAXCHAR + 1];
            let buflen = utf_char2bytes((*menu).mnemonic, buf.as_mut_ptr());
            tv_dict_add_str_len(dict, c"shortcut".as_ptr(), 8, buf.as_ptr(), buflen);
        }

        if !(*menu).actext.is_null() {
            tv_dict_add_str(dict, c"actext".as_ptr(), 6, (*menu).actext);
        }

        if ((*menu).modes & MENU_TIP_MODE) != 0
            && !(*menu).strings[MENU_INDEX_TIP as usize].is_null()
        {
            tv_dict_add_str(
                dict,
                c"tooltip".as_ptr(),
                7,
                (*menu).strings[MENU_INDEX_TIP as usize],
            );
        }

        if (*menu).children.is_null() {
            let commands = tv_dict_alloc();
            tv_dict_add_dict(dict, c"mappings".as_ptr(), 8, commands);

            for (bit, mode_cstr) in MENU_MODE_CHARS.iter().enumerate() {
                if ((*menu).modes & modes & (1 << bit)) != 0 {
                    let impl_dict = tv_dict_alloc();
                    tv_dict_add_allocated_str(
                        impl_dict,
                        c"rhs".as_ptr(),
                        3,
                        str2special_save((*menu).strings[bit], false, kFalse),
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
            menu = (*menu).children;
            while !menu.is_null() {
                let d = menu_get_recursive(menu, modes);
                if tv_dict_len(d) > 0 {
                    tv_list_append_dict(children_list, d);
                }
                menu = (*menu).next;
            }
            tv_dict_add_list(dict, c"submenus".as_ptr(), 8, children_list);
        }
        dict
    }
}

/// Export menus matching path `path_name` (`menu_get`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn menu_get(path_name: *mut c_char, modes: c_int, list: *mut list_T) -> bool {
    unsafe {
        let mut menu = *get_root_menu(path_name);
        if *path_name != NUL {
            menu = find_menu(menu, path_name, modes);
            if menu.is_null() {
                return false;
            }
        }
        while !menu.is_null() {
            let d = menu_get_recursive(menu, modes);
            if !d.is_null() && tv_dict_len(d) > 0 {
                tv_list_append_dict(list, d);
            }
            if *path_name != NUL {
                break;
            }
            menu = (*menu).next;
        }
        true
    }
}

/// Find menu matching `path_name` and `modes` (`find_menu`).
unsafe fn find_menu(
    mut menu: *mut vimmenu_T,
    path_name: *const c_char,
    modes: c_int,
) -> *mut vimmenu_T {
    unsafe {
        debug_assert!(*path_name != NUL);
        let saved_name = xstrdup(path_name);
        let mut name = saved_name;

        while *name != NUL {
            let p = menu_name_skip(name);
            while !menu.is_null() {
                if menu_name_equal(name, menu) {
                    if *p != NUL && (*menu).children.is_null() {
                        emsg(gettext(E_NOTSUBMENU.as_ptr()));
                        xfree(saved_name.cast());
                        return ptr::null_mut();
                    } else if ((*menu).modes & modes) == 0 {
                        emsg(gettext(e_menu_only_exists_in_another_mode.as_ptr()));
                        xfree(saved_name.cast());
                        return ptr::null_mut();
                    } else if *p == NUL {
                        xfree(saved_name.cast());
                        return menu;
                    }
                    break;
                }
                menu = (*menu).next;
            }

            if menu.is_null() {
                semsg!(gettext(E_NOMENU.as_ptr()), name);
                break;
            }
            name = p;
            debug_assert!(*name != NUL);
            menu = (*menu).children;
        }

        xfree(saved_name.cast());
        menu
    }
}

/// Show the mapping associated with a menu item or hierarchy in a sub-menu (`show_menus`).
unsafe fn show_menus(path_name: *mut c_char, modes: c_int) -> c_int {
    unsafe {
        let mut menu: *mut vimmenu_T = ptr::null_mut();
        if *path_name != NUL {
            menu = find_menu(*get_root_menu(path_name), path_name, modes);
            if menu.is_null() {
                return FAIL;
            }
        }

        MENUS_LOCKED += 1;
        msg_puts_title(gettext(c"\n--- Menus ---".as_ptr()));
        show_menus_recursive(menu, modes, 0);
        MENUS_LOCKED -= 1;
        OK
    }
}

/// Recursively show the mappings associated with the menus under the given one (`show_menus_recursive`).
unsafe fn show_menus_recursive(mut menu: *mut vimmenu_T, modes: c_int, mut depth: c_int) {
    unsafe {
        if !menu.is_null() && ((*menu).modes & modes) == 0 {
            return;
        }

        if !menu.is_null() {
            msg_putchar(b'\n' as c_int);
            if got_int {
                return;
            }
            for _ in 0..depth {
                msg_puts(c"  ".as_ptr());
            }
            if (*menu).priority != 0 {
                msg_outnum((*menu).priority);
                msg_puts(c" ".as_ptr());
            }
            msg_outtrans((*menu).name, HLF_D, false);
        }

        if !menu.is_null() && (*menu).children.is_null() {
            for (bit, mode_cstr) in MENU_MODE_CHARS.iter().enumerate() {
                if ((*menu).modes & modes & (1 << bit)) != 0 {
                    msg_putchar(b'\n' as c_int);
                    if got_int {
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
                    if *(*menu).strings[bit] == NUL {
                        msg_puts_hl(c"<Nop>".as_ptr(), HLF_8, false);
                    } else {
                        msg_outtrans_special((*menu).strings[bit], false, 0);
                    }
                }
            }
        } else {
            if menu.is_null() {
                menu = root_menu;
                depth -= 1;
            } else {
                menu = (*menu).children;
            }

            while !menu.is_null() && !got_int {
                if !menu_is_hidden((*menu).dname) {
                    show_menus_recursive(menu, modes, depth + 1);
                }
                menu = (*menu).next;
            }
        }
    }
}

/// Work out what to complete when doing command line completion of menu names (`set_context_in_menu_cmd`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_context_in_menu_cmd(
    xp: *mut expand_T,
    cmd: *const c_char,
    mut arg: *mut c_char,
    forceit: bool,
) -> *mut c_char {
    unsafe {
        let mut path_name: *mut c_char = ptr::null_mut();
        let mut unmenu = false;

        (*xp).xp_context = EXPAND_UNSUCCESSFUL;

        let mut p = arg;
        while *p != NUL {
            if !ascii_isdigit(*p) && *p != b'.' as c_char {
                break;
            }
            p = p.add(1);
        }

        if !ascii_iswhite(*p) {
            if strncmp(arg, c"enable".as_ptr(), 6) == 0
                && (*arg.add(6) == NUL || ascii_iswhite(*arg.add(6)))
            {
                p = arg.add(6);
            } else if strncmp(arg, c"disable".as_ptr(), 7) == 0
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
        EXPAND_EMENU = *cmd == b'e' as c_char;
        if expand_menus && ascii_iswhite(*p) {
            return ptr::null_mut();
        }
        if *p == NUL {
            EXPAND_MODES = get_menu_cmd_modes(cmd, forceit, ptr::null_mut(), &raw mut unmenu);
            if !unmenu {
                EXPAND_MODES = MENU_ALL_MODES;
            }

            let mut menu = root_menu;
            if after_dot > arg {
                let path_len = after_dot.offset_from(arg) as usize;
                path_name = xmalloc(path_len).cast::<c_char>();
                xstrlcpy(path_name, arg, path_len);
            }
            let mut name = path_name;
            while !name.is_null() && *name != NUL {
                p = menu_name_skip(name);
                while !menu.is_null() {
                    if menu_name_equal(name, menu) {
                        if (*p != NUL && (*menu).children.is_null())
                            || ((*menu).modes & EXPAND_MODES) == 0
                        {
                            xfree(path_name.cast());
                            return ptr::null_mut();
                        }
                        break;
                    }
                    menu = (*menu).next;
                }
                if menu.is_null() {
                    xfree(path_name.cast());
                    return ptr::null_mut();
                }
                name = p;
                menu = (*menu).children;
            }
            xfree(path_name.cast());

            (*xp).xp_context = if expand_menus {
                EXPAND_MENUNAMES
            } else {
                EXPAND_MENUS
            };
            (*xp).xp_pattern = after_dot;
            EXPAND_MENU = menu;
        } else {
            (*xp).xp_context = EXPAND_NOTHING;
        }
        ptr::null_mut()
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of (sub)menus (not entries) (`get_menu_name`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_menu_name(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx == 0 {
            GET_MENU_NAME_MENU = EXPAND_MENU;
            GET_MENU_NAME_ADVANCE = false;
        }

        while !GET_MENU_NAME_MENU.is_null()
            && (menu_is_hidden((*GET_MENU_NAME_MENU).dname)
                || menu_is_separator((*GET_MENU_NAME_MENU).dname)
                || (*GET_MENU_NAME_MENU).children.is_null())
        {
            GET_MENU_NAME_MENU = (*GET_MENU_NAME_MENU).next;
        }

        if GET_MENU_NAME_MENU.is_null() {
            return ptr::null_mut();
        }

        let str_ptr = if ((*GET_MENU_NAME_MENU).modes & EXPAND_MODES) != 0 {
            if GET_MENU_NAME_ADVANCE {
                (*GET_MENU_NAME_MENU).en_dname
            } else {
                if (*GET_MENU_NAME_MENU).en_dname.is_null() {
                    GET_MENU_NAME_ADVANCE = true;
                }
                (*GET_MENU_NAME_MENU).dname
            }
        } else {
            c"".as_ptr().cast_mut()
        };

        if GET_MENU_NAME_ADVANCE {
            GET_MENU_NAME_MENU = (*GET_MENU_NAME_MENU).next;
        }

        GET_MENU_NAME_ADVANCE = !GET_MENU_NAME_ADVANCE;

        str_ptr
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of menus and menu entries (`get_menu_names`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_menu_names(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx == 0 {
            GET_MENU_NAMES_MENU = EXPAND_MENU;
            GET_MENU_NAMES_ADVANCE = false;
        }

        while !GET_MENU_NAMES_MENU.is_null() && {
            let dname = (*GET_MENU_NAMES_MENU).dname;
            let dlen = CStr::from_ptr(dname).to_bytes().len();
            menu_is_hidden(dname)
                || (EXPAND_EMENU && menu_is_separator(dname))
                || (dlen > 0 && *dname.add(dlen - 1) == b'.' as c_char)
        } {
            GET_MENU_NAMES_MENU = (*GET_MENU_NAMES_MENU).next;
        }

        if GET_MENU_NAMES_MENU.is_null() {
            return ptr::null_mut();
        }

        let str_ptr = if ((*GET_MENU_NAMES_MENU).modes & EXPAND_MODES) != 0 {
            if !(*GET_MENU_NAMES_MENU).children.is_null() {
                if GET_MENU_NAMES_ADVANCE {
                    xstrlcpy(
                        &raw mut GET_MENU_NAMES_TBUFFER[0],
                        (*GET_MENU_NAMES_MENU).en_dname,
                        TBUFFER_LEN,
                    );
                } else {
                    xstrlcpy(
                        &raw mut GET_MENU_NAMES_TBUFFER[0],
                        (*GET_MENU_NAMES_MENU).dname,
                        TBUFFER_LEN,
                    );
                    if (*GET_MENU_NAMES_MENU).en_dname.is_null() {
                        GET_MENU_NAMES_ADVANCE = true;
                    }
                }
                strcat(&raw mut GET_MENU_NAMES_TBUFFER[0], c"\x01".as_ptr());
                &raw mut GET_MENU_NAMES_TBUFFER[0]
            } else if GET_MENU_NAMES_ADVANCE {
                (*GET_MENU_NAMES_MENU).en_dname
            } else {
                if (*GET_MENU_NAMES_MENU).en_dname.is_null() {
                    GET_MENU_NAMES_ADVANCE = true;
                }
                (*GET_MENU_NAMES_MENU).dname
            }
        } else {
            c"".as_ptr().cast_mut()
        };

        if GET_MENU_NAMES_ADVANCE {
            GET_MENU_NAMES_MENU = (*GET_MENU_NAMES_MENU).next;
        }

        GET_MENU_NAMES_ADVANCE = !GET_MENU_NAMES_ADVANCE;

        str_ptr
    }
}

/// Skip over this element of the menu path and return the start of the next element (`menu_name_skip`).
unsafe fn menu_name_skip(name: *mut c_char) -> *mut c_char {
    unsafe {
        let mut p = name;
        while *p != NUL && *p != b'.' as c_char {
            if *p == b'\\' as c_char || *p == Ctrl_V {
                strmove(p, p.add(1));
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
unsafe fn menu_name_equal(name: *const c_char, menu: *const vimmenu_T) -> bool {
    unsafe {
        if !(*menu).en_name.is_null()
            && (menu_namecmp(name, (*menu).en_name) || menu_namecmp(name, (*menu).en_dname))
        {
            return true;
        }
        menu_namecmp(name, (*menu).name) || menu_namecmp(name, (*menu).dname)
    }
}

unsafe fn menu_namecmp(name: *const c_char, mname: *const c_char) -> bool {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_menu_cmd_modes(
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
unsafe fn popup_mode_name(name: *mut c_char, idx: usize) -> *mut c_char {
    unsafe {
        let len = CStr::from_ptr(name).to_bytes().len();
        debug_assert!(len >= 4);

        let mode_chars = MENU_MODE_CHARS[idx].to_bytes();
        let mode_chars_len = mode_chars.len();
        let p = xstrnsave(name, len + mode_chars_len);
        ptr::copy(p.add(5), p.add(5 + mode_chars_len), len - 4);
        for (i, &b) in mode_chars.iter().enumerate() {
            *p.add(5 + i) = b as c_char;
        }
        p
    }
}

/// Duplicate the menu item text and then process mnemonic key and accelerator text (`menu_text`).
unsafe fn menu_text(
    str: *const c_char,
    mnemonic: *mut c_int,
    actext: *mut *mut c_char,
) -> *mut c_char {
    unsafe {
        let mut p = vim_strchr(str, c_int::from(TAB as u8));
        let text = if !p.is_null() {
            if !actext.is_null() {
                *actext = xstrdup(p.add(1));
            }
            xmemdupz(str.cast(), p.offset_from(str) as usize).cast::<c_char>()
        } else {
            xstrdup(str)
        };

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
                strmove(p, p.add(1));
                p = p.add(1);
            }
        }
        text
    }
}

/// Return true if `name` can be a menu in the MenuBar (`menu_is_menubar`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn menu_is_menubar(name: *const c_char) -> bool {
    unsafe {
        !menu_is_popup(name)
            && !menu_is_toolbar(name)
            && !menu_is_winbar(name)
            && *name != MNU_HIDDEN_CHAR
    }
}

/// Return true if `name` is a popup menu name (`menu_is_popup`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn menu_is_popup(name: *const c_char) -> bool {
    unsafe { strncmp(name, c"PopUp".as_ptr(), 5) == 0 }
}

/// Return true if `name` is a toolbar menu name (`menu_is_toolbar`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn menu_is_toolbar(name: *const c_char) -> bool {
    unsafe { strncmp(name, c"ToolBar".as_ptr(), 7) == 0 }
}

/// Return true if the name is a menu separator identifier (`menu_is_separator`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn menu_is_separator(name: *mut c_char) -> bool {
    unsafe {
        let len = CStr::from_ptr(name).to_bytes().len();
        len > 0 && *name == b'-' as c_char && *name.add(len - 1) == b'-' as c_char
    }
}

/// True if a popup menu or starts with `MNU_HIDDEN_CHAR` (`menu_is_hidden`).
unsafe fn menu_is_hidden(name: *mut c_char) -> bool {
    unsafe { *name == MNU_HIDDEN_CHAR || (menu_is_popup(name) && *name.add(5) != NUL) }
}

unsafe fn get_menu_mode() -> c_int {
    unsafe {
        if (State & MODE_TERMINAL) != 0 {
            return MENU_INDEX_TERMINAL;
        }
        if Visual.active {
            if Visual.select {
                return MENU_INDEX_SELECT;
            }
            return MENU_INDEX_VISUAL;
        }
        if (State & MODE_INSERT) != 0 {
            return MENU_INDEX_INSERT;
        }
        if (State & MODE_CMDLINE) != 0 || State == MODE_ASKMORE || State == MODE_HITRETURN {
            return MENU_INDEX_CMDLINE;
        }
        if finish_op {
            return MENU_INDEX_OP_PENDING;
        }
        if (State & MODE_NORMAL) != 0 {
            return MENU_INDEX_NORMAL;
        }
        if (State & MODE_LANGMAP) != 0 {
            return MENU_INDEX_INSERT;
        }
        MENU_INDEX_INVALID
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_menu_mode_flag() -> c_int {
    unsafe {
        let mode = get_menu_mode();
        if mode == MENU_INDEX_INVALID {
            0
        } else {
            1 << mode
        }
    }
}

/// Display the Special `"PopUp"` menu as a pop-up at the current mouse position (`show_popupmenu`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn show_popupmenu() {
    unsafe {
        let menu_mode = get_menu_mode();
        if menu_mode == MENU_INDEX_INVALID {
            return;
        }
        let mode_cstr = MENU_MODE_CHARS[menu_mode as usize];
        let mode = mode_cstr.as_ptr().cast_mut();
        let mode_len = mode_cstr.to_bytes().len();

        apply_autocmds(EVENT_MENUPOPUP, mode, ptr::null_mut(), false, curbuf);

        let mut menu = root_menu;
        while !menu.is_null() {
            if strncmp(c"PopUp".as_ptr(), (*menu).name, 5) == 0
                && strncmp((*menu).name.add(5), mode, mode_len) == 0
            {
                break;
            }
            menu = (*menu).next;
        }

        if menu.is_null() || (*menu).children.is_null() {
            return;
        }

        pum_show_popupmenu(menu);
    }
}

/// Execute `menu` (`execute_menu`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execute_menu(eap: *const exarg_T, menu: *mut vimmenu_T, mode_idx: c_int) {
    unsafe {
        let mut idx = mode_idx;

        if idx < 0 {
            if (State & MODE_TERMINAL) != 0 {
                idx = MENU_INDEX_TERMINAL;
            } else if (State & MODE_CMDLINE) != 0 {
                idx = MENU_INDEX_CMDLINE;
            } else if (get_real_state() & MODE_VISUAL) != 0 {
                idx = MENU_INDEX_VISUAL;
            } else if ((State & MODE_INSERT) != 0 || restart_edit != 0) && current_sctx.sc_sid == 0
            {
                idx = MENU_INDEX_INSERT;
            } else if !eap.is_null() && (*eap).addr_count != 0 {
                idx = MENU_INDEX_VISUAL;

                let tpos: pos_T;
                if (*curbuf).b_visual.vi_start.lnum == (*eap).line1
                    && (*curbuf).b_visual.vi_end.lnum == (*eap).line2
                {
                    Visual.mode = (*curbuf).b_visual.vi_mode;
                    tpos = (*curbuf).b_visual.vi_end;
                    (*curwin).w_cursor = (*curbuf).b_visual.vi_start;
                    (*curwin).w_curswant = (*curbuf).b_visual.vi_curswant;
                } else {
                    Visual.mode = b'V' as c_int;
                    (*curwin).w_cursor.lnum = (*eap).line1;
                    (*curwin).w_cursor.col = 1;
                    tpos = pos_T {
                        lnum: (*eap).line2,
                        col: MAXCOL,
                        coladd: 0,
                    };
                }

                Visual.active = true;
                Visual.reselect = 1;
                check_cursor(curwin);
                Visual.start = (*curwin).w_cursor;
                (*curwin).w_cursor = tpos;

                check_cursor(curwin);

                if *p_sel == b'e' as c_char && gchar_cursor() != c_int::from(NUL) {
                    (*curwin).w_cursor.col += 1;
                }
            }
        }

        if idx == MENU_INDEX_INVALID || eap.is_null() {
            idx = MENU_INDEX_NORMAL;
        }

        let u_idx = idx as usize;
        if !(*menu).strings[u_idx].is_null() && ((*menu).modes & (1 << idx)) != 0 {
            if eap.is_null() || current_sctx.sc_sid != 0 {
                let mut save_state: save_state_T = std::mem::zeroed();
                ex_normal_busy += 1;
                save_current_state(&raw mut save_state);
                exec_normal_cmd(
                    (*menu).strings[u_idx],
                    (*menu).noremap[u_idx],
                    (*menu).silent[u_idx],
                );
                restore_current_state(&raw mut save_state);
                ex_normal_busy -= 1;
            } else {
                ins_typebuf(
                    (*menu).strings[u_idx],
                    (*menu).noremap[u_idx],
                    0,
                    true,
                    (*menu).silent[u_idx],
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
unsafe fn menu_getbyname(name_arg: *mut c_char) -> *mut vimmenu_T {
    unsafe {
        let saved_name = xstrdup(name_arg);
        let mut menu = *get_root_menu(saved_name);
        let mut name = saved_name;
        let mut gave_emsg = false;
        while *name != NUL {
            let p = menu_name_skip(name);

            while !menu.is_null() {
                if menu_name_equal(name, menu) {
                    if *p == NUL && !(*menu).children.is_null() {
                        emsg(gettext(
                            c"E333: Menu path must lead to a menu item".as_ptr(),
                        ));
                        gave_emsg = true;
                        menu = ptr::null_mut();
                    } else if *p != NUL && (*menu).children.is_null() {
                        emsg(gettext(E_NOTSUBMENU.as_ptr()));
                        menu = ptr::null_mut();
                    }
                    break;
                }
                menu = (*menu).next;
            }
            if menu.is_null() || *p == NUL {
                break;
            }
            menu = (*menu).children;
            name = p;
        }
        xfree(saved_name.cast());
        if menu.is_null() {
            if !gave_emsg {
                semsg!(gettext(c"E334: Menu not found: %s".as_ptr()), name_arg);
            }
            return ptr::null_mut();
        }

        menu
    }
}

/// `:emenu` (`ex_emenu`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_emenu(eap: *mut exarg_T) {
    unsafe {
        let mut arg = (*eap).arg;
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

        execute_menu(eap, menu, mode_idx);
    }
}

/// Given a menu descriptor, e.g. `"File.New"`, find it in the menu hierarchy (`menu_find`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn menu_find(path_name: *const c_char) -> *mut vimmenu_T {
    unsafe {
        let mut menu = *get_root_menu(path_name);
        let saved_name = xstrdup(path_name);
        let mut name = saved_name;
        while *name != NUL {
            let p = menu_name_skip(name);

            while !menu.is_null() {
                if menu_name_equal(name, menu) {
                    if (*menu).children.is_null() {
                        if *p == NUL {
                            emsg(gettext(c"E336: Menu path must lead to a sub-menu".as_ptr()));
                        } else {
                            emsg(gettext(E_NOTSUBMENU.as_ptr()));
                        }
                        xfree(saved_name.cast());
                        return ptr::null_mut();
                    }
                    if *p == NUL {
                        xfree(saved_name.cast());
                        return menu;
                    }
                    break;
                }
                menu = (*menu).next;
            }
            if menu.is_null() {
                break;
            }

            menu = (*menu).children;
            name = p;
        }

        if menu.is_null() {
            emsg(gettext(c"E337: Menu not found - check menu names".as_ptr()));
        }
        xfree(saved_name.cast());
        menu
    }
}

/// `:menutrans` (`ex_menutranslate`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_menutranslate(eap: *mut exarg_T) {
    unsafe {
        let mut arg = (*eap).arg;

        if MENUTRANS_GA.ga_itemsize == 0 {
            ga_init(&raw mut MENUTRANS_GA, size_of::<menutrans_T>() as c_int, 5);
        }

        if strncmp(arg, c"clear".as_ptr(), 5) == 0
            && ends_excmd(c_int::from(*skipwhite(arg.add(5)) as u8)) != 0
        {
            for i in 0..MENUTRANS_GA.ga_len {
                let mt = MENUTRANS_GA.ga_data.cast::<menutrans_T>().add(i as usize);
                xfree((*mt).from.cast());
                xfree((*mt).from_noamp.cast());
                xfree((*mt).to.cast());
            }
            ga_clear(&raw mut MENUTRANS_GA);
            del_menutrans_vars();
        } else {
            let mut from = arg;
            arg = menu_skip_part(arg);
            let mut to = skipwhite(arg);
            *arg = NUL;
            arg = menu_skip_part(to);
            if arg == to {
                emsg(gettext(e_invarg.as_ptr()));
            } else {
                from = xstrdup(from);
                let from_noamp = menu_text(from, ptr::null_mut(), ptr::null_mut());
                to = xmemdupz(to.cast(), arg.offset_from(to) as usize).cast();
                menu_translate_tab_and_shift(from);
                menu_translate_tab_and_shift(to);
                menu_unescape_name(from);
                menu_unescape_name(to);
                ga_grow(&raw mut MENUTRANS_GA, 1);
                let tp = MENUTRANS_GA
                    .ga_data
                    .cast::<menutrans_T>()
                    .add(MENUTRANS_GA.ga_len as usize);
                MENUTRANS_GA.ga_len += 1;
                (*tp).from = from;
                (*tp).from_noamp = from_noamp;
                (*tp).to = to;
            }
        }
    }
}

/// Find the character just after one part of a menu name (`menu_skip_part`).
unsafe fn menu_skip_part(mut p: *mut c_char) -> *mut c_char {
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
unsafe fn menutrans_lookup(name: *mut c_char, len: c_int) -> *mut c_char {
    unsafe {
        let tp = MENUTRANS_GA.ga_data.cast::<menutrans_T>();
        let u_len = len as usize;

        for i in 0..MENUTRANS_GA.ga_len {
            let entry = tp.add(i as usize);
            if vim_strnicmp_asc(name, (*entry).from, u_len) == 0 && *(*entry).from.add(u_len) == NUL
            {
                return (*entry).to;
            }
        }

        let c = *name.add(u_len);
        *name.add(u_len) = NUL;
        let dname = menu_text(name, ptr::null_mut(), ptr::null_mut());
        *name.add(u_len) = c;
        for i in 0..MENUTRANS_GA.ga_len {
            let entry = tp.add(i as usize);
            if vim_stricmp(dname, (*entry).from_noamp) == 0 {
                xfree(dname.cast());
                return (*entry).to;
            }
        }
        xfree(dname.cast());

        ptr::null_mut()
    }
}

/// Unescape the name in the translate dictionary table (`menu_unescape_name`).
unsafe fn menu_unescape_name(name: *mut c_char) {
    unsafe {
        let mut p = name;
        while *p != NUL && *p != b'.' as c_char {
            if *p == b'\\' as c_char {
                strmove(p, p.add(1));
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
    }
}

/// Isolate the menu name (`menu_translate_tab_and_shift`).
unsafe fn menu_translate_tab_and_shift(arg_start: *mut c_char) -> *mut c_char {
    unsafe {
        let mut arg = arg_start;

        while *arg != NUL && !ascii_iswhite(*arg) {
            if (*arg == b'\\' as c_char || *arg == Ctrl_V) && *arg.add(1) != NUL {
                arg = arg.add(1);
            } else if vim_strnicmp_asc(arg, c"<TAB>".as_ptr(), 5) == 0 {
                *arg = TAB;
                strmove(arg.add(1), arg.add(5));
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
unsafe fn menuitem_getinfo(
    menu_name: *const c_char,
    menu: *const vimmenu_T,
    modes: c_int,
    dict: *mut dict_T,
) {
    unsafe {
        if *menu_name == NUL {
            let l = tv_list_alloc(kListLenMayKnow);
            tv_dict_add_list(dict, c"submenus".as_ptr(), 8, l);
            let mut topmenu = menu;
            while !topmenu.is_null() {
                if !menu_is_hidden((*topmenu).dname) {
                    tv_list_append_string(l, (*topmenu).dname, -1);
                }
                topmenu = (*topmenu).next;
            }
            return;
        }

        tv_dict_add_str(dict, c"name".as_ptr(), 4, (*menu).name);
        tv_dict_add_str(dict, c"display".as_ptr(), 7, (*menu).dname);
        if !(*menu).actext.is_null() {
            tv_dict_add_str(dict, c"accel".as_ptr(), 5, (*menu).actext);
        }
        tv_dict_add_nr(dict, c"priority".as_ptr(), 8, i64::from((*menu).priority));
        tv_dict_add_str(dict, c"modes".as_ptr(), 5, get_menu_mode_str((*menu).modes));

        let mut buf = [0 as c_char; NUMBUFLEN];
        let buflen = utf_char2bytes((*menu).mnemonic, buf.as_mut_ptr());
        buf[buflen as usize] = NUL;
        tv_dict_add_str_len(dict, c"shortcut".as_ptr(), 8, buf.as_ptr(), buflen);

        if (*menu).children.is_null() {
            let mut bit = 0usize;
            while bit < MENU_MODES && ((1 << bit) & modes) == 0 {
                bit += 1;
            }

            if bit < MENU_MODES {
                if !(*menu).strings[bit].is_null() {
                    let rhs = if *(*menu).strings[bit] == NUL {
                        xmemdupz(c"<Nop>".as_ptr().cast(), 5).cast()
                    } else {
                        str2special_save((*menu).strings[bit], false, kFalse)
                    };
                    tv_dict_add_allocated_str(dict, c"rhs".as_ptr(), 3, rhs);
                }
                tv_dict_add_bool(
                    dict,
                    c"noremenu".as_ptr(),
                    8,
                    if (*menu).noremap[bit] == REMAP_NONE {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
                tv_dict_add_bool(
                    dict,
                    c"script".as_ptr(),
                    6,
                    if (*menu).noremap[bit] == REMAP_SCRIPT {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
                tv_dict_add_bool(
                    dict,
                    c"silent".as_ptr(),
                    6,
                    if (*menu).silent[bit] {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
                tv_dict_add_bool(
                    dict,
                    c"enabled".as_ptr(),
                    7,
                    if ((*menu).enabled & (1 << bit)) != 0 {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                );
            }
        } else {
            let l = tv_list_alloc(kListLenMayKnow);
            tv_dict_add_list(dict, c"submenus".as_ptr(), 8, l);
            let mut child: *const vimmenu_T = (*menu).children;
            while !child.is_null() {
                tv_list_append_string(l, (*child).dname, -1);
                child = (*child).next;
            }
        }
    }
}

/// `menu_info()` function (`f_menu_info`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_menu_info(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        let retdict = (*rettv).vval.v_dict;

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

        let mut menu: *const vimmenu_T = *get_root_menu(menu_name);
        let saved_name = xstrdup(menu_name);
        if *saved_name != NUL {
            let mut name = saved_name;
            while *name != NUL {
                let p = menu_name_skip(name);
                while !menu.is_null() {
                    if menu_name_equal(name, menu) {
                        break;
                    }
                    menu = (*menu).next;
                }
                if menu.is_null() || *p == NUL {
                    break;
                }
                menu = (*menu).children;
                name = p;
            }
        }
        xfree(saved_name.cast());

        if menu.is_null() {
            return;
        }

        if ((*menu).modes & modes) != 0 {
            menuitem_getinfo(menu_name, menu, modes, retdict);
        }
    }
}
