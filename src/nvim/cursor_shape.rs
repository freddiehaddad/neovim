//! Handling of cursor and mouse pointer shapes in various modes (`src/nvim/cursor_shape.c`).

#![allow(non_camel_case_types, non_snake_case, clippy::missing_safety_doc)]

#[cfg(not(test))]
use crate::api::private::helpers::{arena_array, arena_dict};
use crate::api::private::validate::Array;
#[cfg(not(test))]
use crate::api::private::validate::{Dict, KeyValuePair, Object, ObjectData, ObjectType};
#[cfg(not(test))]
use crate::charset::getdigits_int;
#[cfg(not(test))]
use crate::ex_getln::{cmdline_at_end, cmdline_overstrike};
#[cfg(not(test))]
use crate::globals::*;
#[cfg(not(test))]
use crate::highlight_group::{syn_check_group, syn_id2attr};
#[cfg(not(test))]
use crate::log::{LOGLVL_WRN, logmsg};
#[cfg(not(test))]
use crate::map_glyph_cache::NvimString;
use crate::memory::Arena;
#[cfg(not(test))]
use crate::option_vars::*;
#[cfg(not(test))]
use crate::ui::ui_mode_info_set;
use std::ffi::{CStr, c_char, c_int};

pub const SHAPE_IDX_N: usize = 0;
pub const SHAPE_IDX_V: usize = 1;
pub const SHAPE_IDX_I: usize = 2;
pub const SHAPE_IDX_R: usize = 3;
pub const SHAPE_IDX_C: usize = 4;
pub const SHAPE_IDX_CI: usize = 5;
pub const SHAPE_IDX_CR: usize = 6;
pub const SHAPE_IDX_O: usize = 7;
pub const SHAPE_IDX_VE: usize = 8;
pub const SHAPE_IDX_CLINE: usize = 9;
pub const SHAPE_IDX_STATUS: usize = 10;
pub const SHAPE_IDX_SDRAG: usize = 11;
pub const SHAPE_IDX_VSEP: usize = 12;
pub const SHAPE_IDX_VDRAG: usize = 13;
pub const SHAPE_IDX_MORE: usize = 14;
pub const SHAPE_IDX_MOREL: usize = 15;
pub const SHAPE_IDX_SM: usize = 16;
pub const SHAPE_IDX_TERM: usize = 17;
pub const SHAPE_IDX_COUNT: usize = 18;

pub const SHAPE_MOUSE: c_int = 1;
pub const SHAPE_CURSOR: c_int = 2;
const SHAPE_BOTH: c_char = (SHAPE_CURSOR + SHAPE_MOUSE) as c_char;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    SHAPE_BLOCK = 0,
    SHAPE_HOR = 1,
    SHAPE_VER = 2,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct cursorentry_T {
    pub full_name: *const c_char,
    pub shape: CursorShape,
    pub mshape: c_int,
    pub percentage: c_int,
    pub blinkwait: c_int,
    pub blinkon: c_int,
    pub blinkoff: c_int,
    pub id: c_int,
    pub id_lm: c_int,
    pub name: *const c_char,
    pub used_for: c_char,
}

unsafe impl Sync for cursorentry_T {}

pub use crate::api::buffer::VisualState;
pub use crate::eval::deprecated::pos_T;
#[cfg(not(test))]
use crate::state::{
    MODE_CMDLINE, MODE_INSERT, MODE_SHOWMATCH, MODE_TERMINAL, REPLACE_FLAG, VREPLACE_FLAG,
};

const fn entry(
    full_name: &'static CStr,
    blinkwait: c_int,
    blinkon: c_int,
    blinkoff: c_int,
    name: &'static CStr,
    used_for: c_char,
) -> cursorentry_T {
    cursorentry_T {
        full_name: full_name.as_ptr(),
        shape: CursorShape::SHAPE_BLOCK,
        mshape: 0,
        percentage: 0,
        blinkwait,
        blinkon,
        blinkoff,
        id: 0,
        id_lm: 0,
        name: name.as_ptr(),
        used_for,
    }
}

#[unsafe(no_mangle)]
pub static mut shape_table: [cursorentry_T; SHAPE_IDX_COUNT] = [
    entry(c"normal", 700, 400, 250, c"n", SHAPE_BOTH),
    entry(c"visual", 700, 400, 250, c"v", SHAPE_BOTH),
    entry(c"insert", 700, 400, 250, c"i", SHAPE_BOTH),
    entry(c"replace", 700, 400, 250, c"r", SHAPE_BOTH),
    entry(c"cmdline_normal", 700, 400, 250, c"c", SHAPE_BOTH),
    entry(c"cmdline_insert", 700, 400, 250, c"ci", SHAPE_BOTH),
    entry(c"cmdline_replace", 700, 400, 250, c"cr", SHAPE_BOTH),
    entry(c"operator", 700, 400, 250, c"o", SHAPE_BOTH),
    entry(c"visual_select", 700, 400, 250, c"ve", SHAPE_BOTH),
    entry(c"cmdline_hover", 0, 0, 0, c"e", SHAPE_MOUSE as c_char),
    entry(c"statusline_hover", 0, 0, 0, c"s", SHAPE_MOUSE as c_char),
    entry(c"statusline_drag", 0, 0, 0, c"sd", SHAPE_MOUSE as c_char),
    entry(c"vsep_hover", 0, 0, 0, c"vs", SHAPE_MOUSE as c_char),
    entry(c"vsep_drag", 0, 0, 0, c"vd", SHAPE_MOUSE as c_char),
    entry(c"more", 0, 0, 0, c"m", SHAPE_MOUSE as c_char),
    entry(c"more_lastline", 0, 0, 0, c"ml", SHAPE_MOUSE as c_char),
    entry(c"showmatch", 100, 100, 100, c"sm", SHAPE_CURSOR as c_char),
    entry(c"terminal", 0, 0, 0, c"t", SHAPE_CURSOR as c_char),
];

#[cfg(not(test))]
#[inline]
unsafe fn cstr_as_nvim_string(s: *const c_char) -> NvimString {
    let size = if s.is_null() {
        0
    } else {
        unsafe { CStr::from_ptr(s).to_bytes().len() }
    };
    NvimString {
        data: s.cast_mut(),
        size,
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn dict_put_c(dic: &mut Dict, key: &'static CStr, value: Object) {
    assert!(dic.size < dic.capacity);
    let key_bytes = key.to_bytes();
    unsafe {
        *dic.items.add(dic.size) = KeyValuePair {
            key: NvimString {
                data: key.as_ptr().cast_mut(),
                size: key_bytes.len(),
            },
            value,
        };
    }
    dic.size += 1;
}

#[cfg(not(test))]
#[inline]
unsafe fn strchr_ascii(mut s: *const c_char, ch: u8) -> *const c_char {
    unsafe {
        loop {
            let b = *s as u8;
            if b == ch {
                return s;
            }
            if b == 0 {
                return std::ptr::null();
            }
            s = s.add(1);
        }
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn strnicmp_asc(mut s1: *const c_char, mut s2: *const c_char, mut len: usize) -> bool {
    unsafe {
        while len > 0 {
            let c1 = (*s1 as u8).to_ascii_lowercase();
            let c2 = (*s2 as u8).to_ascii_lowercase();
            if c1 != c2 {
                return false;
            }
            if c1 == 0 {
                break;
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
            len -= 1;
        }
        true
    }
}

#[cfg(not(test))]
unsafe fn clear_shape_table() {
    let tbl = unsafe { &mut *std::ptr::addr_of_mut!(shape_table) };
    for entry in tbl.iter_mut() {
        entry.shape = CursorShape::SHAPE_BLOCK;
        entry.blinkwait = 0;
        entry.blinkon = 0;
        entry.blinkoff = 0;
        entry.id = 0;
        entry.id_lm = 0;
    }
}

/// Converts `shape_table` into an `Array` of `Dict`s (`mode_style_array`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mode_style_array(arena: *mut Arena) -> Array {
    #[cfg(test)]
    {
        let _ = arena;
        Array {
            size: 0,
            capacity: 0,
            items: std::ptr::null_mut(),
        }
    }
    #[cfg(not(test))]
    unsafe {
        let mut all = arena_array(arena, SHAPE_IDX_COUNT);
        let tbl = &*std::ptr::addr_of!(shape_table);

        for cur in tbl.iter() {
            let is_cursor = (cur.used_for as c_int & SHAPE_CURSOR) != 0;
            let mut dic = arena_dict(arena, 3 + if is_cursor { 9 } else { 0 });
            dict_put_c(
                &mut dic,
                c"name",
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: cstr_as_nvim_string(cur.full_name),
                    },
                },
            );
            dict_put_c(
                &mut dic,
                c"short_name",
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: cstr_as_nvim_string(cur.name),
                    },
                },
            );
            if (cur.used_for as c_int & SHAPE_MOUSE) != 0 {
                dict_put_c(
                    &mut dic,
                    c"mouse_shape",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: cur.mshape as i64,
                        },
                    },
                );
            }
            if is_cursor {
                let shape_cstr = match cur.shape {
                    CursorShape::SHAPE_BLOCK => c"block",
                    CursorShape::SHAPE_VER => c"vertical",
                    CursorShape::SHAPE_HOR => c"horizontal",
                };
                dict_put_c(
                    &mut dic,
                    c"cursor_shape",
                    Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData {
                            string: cstr_as_nvim_string(shape_cstr.as_ptr()),
                        },
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"cell_percentage",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: cur.percentage as i64,
                        },
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"blinkwait",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: cur.blinkwait as i64,
                        },
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"blinkon",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: cur.blinkon as i64,
                        },
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"blinkoff",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: cur.blinkoff as i64,
                        },
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"hl_id",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: cur.id as i64,
                        },
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"id_lm",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: cur.id_lm as i64,
                        },
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"attr_id",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: if cur.id != 0 {
                                syn_id2attr(cur.id) as i64
                            } else {
                                0
                            },
                        },
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"attr_id_lm",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: if cur.id_lm != 0 {
                                syn_id2attr(cur.id_lm) as i64
                            } else {
                                0
                            },
                        },
                    },
                );
            }
            assert!(all.size < all.capacity);
            *all.items.add(all.size) = Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData { dict: dic },
            };
            all.size += 1;
        }
        all
    }
}

/// Parses the `'guicursor'` option (`parse_shape_opt`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parse_shape_opt(what: c_int) -> *const c_char {
    #[cfg(test)]
    {
        let _ = what;
        std::ptr::null()
    }
    #[cfg(not(test))]
    unsafe {
        let mut p: *const c_char = std::ptr::null();
        let mut idx: usize = 0;
        let mut found_ve = false;
        let tbl = &mut *std::ptr::addr_of_mut!(shape_table);

        for round in 1..=2 {
            if round == 2 || *p_guicursor == 0 {
                clear_shape_table();
                if *p_guicursor == 0 {
                    ui_mode_info_set();
                    return std::ptr::null();
                }
            }
            let mut modep: *const c_char = p_guicursor;
            while !modep.is_null() && *modep != 0 {
                let colonp = strchr_ascii(modep, b':');
                let commap = strchr_ascii(modep, b',');

                if colonp.is_null() || (!commap.is_null() && (commap as usize) < (colonp as usize))
                {
                    return c"E545: Missing colon".as_ptr();
                }
                if colonp == modep {
                    return c"E546: Illegal mode".as_ptr();
                }

                let mut all_idx: isize = -1;
                while (modep as usize) < (colonp as usize) || all_idx >= 0 {
                    if all_idx < 0 {
                        let m1 = *modep.add(1) as u8;
                        let len = if m1 == b'-' || m1 == b':' {
                            1usize
                        } else {
                            2usize
                        };

                        if len == 1 && (*modep as u8).eq_ignore_ascii_case(&b'a') {
                            all_idx = (SHAPE_IDX_COUNT - 1) as isize;
                        } else {
                            idx = SHAPE_IDX_COUNT;
                            for (i, entry) in tbl.iter().enumerate() {
                                if strnicmp_asc(modep, entry.name, len) {
                                    idx = i;
                                    break;
                                }
                            }
                            if idx == SHAPE_IDX_COUNT || (tbl[idx].used_for as c_int & what) == 0 {
                                return c"E546: Illegal mode".as_ptr();
                            }
                            if len == 2 && *modep as u8 == b'v' && m1 == b'e' {
                                found_ve = true;
                            }
                        }
                        modep = modep.add(len + 1);
                    }

                    if all_idx >= 0 {
                        idx = all_idx as usize;
                        all_idx -= 1;
                    }

                    p = colonp.add(1);
                    while *p != 0 && *p as u8 != b',' {
                        let first_byte = *p as u8;
                        let len = if strnicmp_asc(p, c"ver".as_ptr(), 3)
                            || strnicmp_asc(p, c"hor".as_ptr(), 3)
                        {
                            3usize
                        } else if strnicmp_asc(p, c"blinkwait".as_ptr(), 9) {
                            9usize
                        } else if strnicmp_asc(p, c"blinkon".as_ptr(), 7) {
                            7usize
                        } else if strnicmp_asc(p, c"blinkoff".as_ptr(), 8) {
                            8usize
                        } else {
                            0usize
                        };

                        if len != 0 {
                            p = p.add(len);
                            if !(*p as u8).is_ascii_digit() {
                                return c"E548: Digit expected".as_ptr();
                            }
                            let n = getdigits_int((&raw mut p).cast(), false, 0);
                            if len == 3 {
                                if n == 0 {
                                    return c"E549: Illegal percentage".as_ptr();
                                }
                                if round == 2 {
                                    if first_byte.eq_ignore_ascii_case(&b'v') {
                                        tbl[idx].shape = CursorShape::SHAPE_VER;
                                    } else {
                                        tbl[idx].shape = CursorShape::SHAPE_HOR;
                                    }
                                    tbl[idx].percentage = n;
                                }
                            } else if round == 2 {
                                if len == 9 {
                                    tbl[idx].blinkwait = n;
                                } else if len == 7 {
                                    tbl[idx].blinkon = n;
                                } else {
                                    tbl[idx].blinkoff = n;
                                }
                            }
                        } else if strnicmp_asc(p, c"block".as_ptr(), 5) {
                            if round == 2 {
                                tbl[idx].shape = CursorShape::SHAPE_BLOCK;
                            }
                            p = p.add(5);
                        } else {
                            let mut endp = strchr_ascii(p, b'-');
                            if commap.is_null() {
                                if endp.is_null() {
                                    endp = p.add(CStr::from_ptr(p).to_bytes().len());
                                }
                            } else if endp.is_null() || (endp as usize) > (commap as usize) {
                                endp = commap;
                            }
                            let slashp = strchr_ascii(p, b'/');
                            let mut saved_hl_id = first_byte as c_int;
                            if !slashp.is_null() && (slashp as usize) < (endp as usize) {
                                saved_hl_id = syn_check_group(p, (slashp as usize) - (p as usize));
                                p = slashp.add(1);
                            }
                            if round == 2 {
                                tbl[idx].id = syn_check_group(p, (endp as usize) - (p as usize));
                                tbl[idx].id_lm = tbl[idx].id;
                                if !slashp.is_null() && (slashp as usize) < (endp as usize) {
                                    tbl[idx].id = saved_hl_id;
                                }
                            }
                            p = endp;
                        }

                        if *p as u8 == b'-' {
                            p = p.add(1);
                        }
                    }
                }
                modep = p;
                if !modep.is_null() && *modep as u8 == b',' {
                    modep = modep.add(1);
                }
            }
        }

        if !found_ve {
            let v_entry = tbl[SHAPE_IDX_V];
            tbl[SHAPE_IDX_VE].shape = v_entry.shape;
            tbl[SHAPE_IDX_VE].percentage = v_entry.percentage;
            tbl[SHAPE_IDX_VE].blinkwait = v_entry.blinkwait;
            tbl[SHAPE_IDX_VE].blinkon = v_entry.blinkon;
            tbl[SHAPE_IDX_VE].blinkoff = v_entry.blinkoff;
            tbl[SHAPE_IDX_VE].id = v_entry.id;
            tbl[SHAPE_IDX_VE].id_lm = v_entry.id_lm;
        }
        ui_mode_info_set();
        std::ptr::null()
    }
}

/// Map cursor mode from string to integer (`cursor_mode_str2int`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cursor_mode_str2int(mode: *const c_char) -> c_int {
    let mode_bytes = unsafe { CStr::from_ptr(mode).to_bytes() };
    let tbl = unsafe { &*std::ptr::addr_of!(shape_table) };
    for (mode_idx, entry) in tbl.iter().enumerate() {
        let full_bytes = unsafe { CStr::from_ptr(entry.full_name).to_bytes() };
        if full_bytes == mode_bytes {
            return mode_idx as c_int;
        }
    }
    #[cfg(not(test))]
    unsafe {
        logmsg!(
            LOGLVL_WRN,
            std::ptr::null(),
            c"cursor_mode_str2int".as_ptr(),
            284,
            true,
            c"Unknown mode %s".as_ptr(),
            mode,
        );
    }
    -1
}

/// Check if a syntax id is used as a cursor style (`cursor_mode_uses_syn_id`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cursor_mode_uses_syn_id(syn_id: c_int) -> bool {
    #[cfg(test)]
    {
        let _ = syn_id;
        false
    }
    #[cfg(not(test))]
    unsafe {
        if *p_guicursor == 0 {
            return false;
        }
        let tbl = &*std::ptr::addr_of!(shape_table);
        for entry in tbl.iter() {
            if entry.id == syn_id || entry.id_lm == syn_id {
                return true;
            }
        }
        false
    }
}

/// Return the index into `shape_table[]` for the current mode (`cursor_get_mode_idx`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cursor_get_mode_idx() -> c_int {
    #[cfg(test)]
    {
        SHAPE_IDX_N as c_int
    }
    #[cfg(not(test))]
    unsafe {
        if State == MODE_SHOWMATCH {
            SHAPE_IDX_SM as c_int
        } else if State == MODE_TERMINAL {
            SHAPE_IDX_TERM as c_int
        } else if (State & VREPLACE_FLAG) != 0 || (State & REPLACE_FLAG) != 0 {
            SHAPE_IDX_R as c_int
        } else if (State & MODE_INSERT) != 0 {
            SHAPE_IDX_I as c_int
        } else if (State & MODE_CMDLINE) != 0 {
            if cmdline_at_end() {
                SHAPE_IDX_C as c_int
            } else if cmdline_overstrike() {
                SHAPE_IDX_CR as c_int
            } else {
                SHAPE_IDX_CI as c_int
            }
        } else if finish_op {
            SHAPE_IDX_O as c_int
        } else if Visual.active {
            if *p_sel as u8 == b'e' {
                SHAPE_IDX_VE as c_int
            } else {
                SHAPE_IDX_V as c_int
            }
        } else {
            SHAPE_IDX_N as c_int
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_mode_str2int_finds_modes() {
        unsafe {
            assert_eq!(
                cursor_mode_str2int(c"normal".as_ptr()),
                SHAPE_IDX_N as c_int
            );
            assert_eq!(
                cursor_mode_str2int(c"terminal".as_ptr()),
                SHAPE_IDX_TERM as c_int
            );
            assert_eq!(cursor_mode_str2int(c"nonexistent".as_ptr()), -1);
        }
    }
}
