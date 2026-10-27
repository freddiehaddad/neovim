//! Handling of cursor and mouse pointer shapes in various modes (`src/nvim/cursor_shape.c`).

use crate::api::private::helpers::{arena_array, arena_dict, cstr_as_string};
use crate::api::private::validate::{
    Array, Dict, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::charset::getdigits_int;
use crate::ex_getln::{cmdline_at_end, cmdline_overstrike};
use crate::globals::*;
use crate::highlight_group::{syn_check_group, syn_id2attr};
#[cfg(test)]
use crate::log::{LOGLVL_WRN, logmsg};
use crate::memory::Arena;
use crate::option_vars::*;
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
    pub full_name: &'static CStr,
    pub shape: CursorShape,
    pub mshape: c_int,
    pub percentage: c_int,
    pub blinkwait: c_int,
    pub blinkon: c_int,
    pub blinkoff: c_int,
    pub id: c_int,
    pub id_lm: c_int,
    pub name: &'static CStr,
    pub used_for: c_char,
}

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
        full_name,
        shape: CursorShape::SHAPE_BLOCK,
        mshape: 0,
        percentage: 0,
        blinkwait,
        blinkon,
        blinkoff,
        id: 0,
        id_lm: 0,
        name,
        used_for,
    }
}

static SHAPE_TABLE: GlobalState<[cursorentry_T; SHAPE_IDX_COUNT]> = GlobalState::new([
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
]);

#[inline]
pub fn shape_table_mut() -> &'static mut [cursorentry_T; SHAPE_IDX_COUNT] {
    unsafe { &mut *SHAPE_TABLE.as_ptr() }
}

#[inline]
fn dict_put_c(dic: &mut Dict, key: &'static CStr, value: Object) {
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


fn clear_shape_table() {
    let tbl = unsafe { &mut *std::ptr::addr_of_mut!((*shape_table_mut())) };
    for entry in tbl.iter_mut() {
        entry.shape = CursorShape::SHAPE_BLOCK;
        entry.blinkwait = 0;
        entry.blinkon = 0;
        entry.blinkoff = 0;
        entry.id = 0;
        entry.id_lm = 0;
    }
}

/// Converts `(*shape_table_mut())` into an `Array` of `Dict`s (`mode_style_array`).
pub fn mode_style_array(arena: *mut Arena) -> Array {
    unsafe {
        let mut all = arena_array(arena, SHAPE_IDX_COUNT);
        let tbl = &*std::ptr::addr_of!((*shape_table_mut()));

        for cur in tbl.iter() {
            let is_cursor = (cur.used_for as c_int & SHAPE_CURSOR) != 0;
            let mut dic = arena_dict(arena, 3 + if is_cursor { 9 } else { 0 });
            dict_put_c(
                &mut dic,
                c"name",
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData::String(cstr_as_string(cur.full_name.as_ptr())),
                },
            );
            dict_put_c(
                &mut dic,
                c"short_name",
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData::String(cstr_as_string(cur.name.as_ptr())),
                },
            );
            if (cur.used_for as c_int & SHAPE_MOUSE) != 0 {
                dict_put_c(
                    &mut dic,
                    c"mouse_shape",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(cur.mshape as i64),
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
                        data: ObjectData::String(cstr_as_string(shape_cstr.as_ptr())),
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"cell_percentage",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(cur.percentage as i64),
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"blinkwait",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(cur.blinkwait as i64),
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"blinkon",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(cur.blinkon as i64),
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"blinkoff",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(cur.blinkoff as i64),
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"hl_id",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(cur.id as i64),
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"id_lm",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(cur.id_lm as i64),
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"attr_id",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(if cur.id != 0 {
                            syn_id2attr(cur.id) as i64
                        } else {
                            0
                        }),
                    },
                );
                dict_put_c(
                    &mut dic,
                    c"attr_id_lm",
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(if cur.id_lm != 0 {
                            syn_id2attr(cur.id_lm) as i64
                        } else {
                            0
                        }),
                    },
                );
            }
            assert!(all.size < all.capacity);
            *all.items.add(all.size) = Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData::Dict(dic),
            };
            all.size += 1;
        }
        all
    }
}

/// Parses the `'guicursor'` option (`parse_shape_opt`).
pub fn parse_shape_opt(what: c_int) -> *const c_char {
    unsafe {
        let mut p: *const c_char = std::ptr::null();
        let mut idx: usize = 0;
        let mut found_ve = false;
        let tbl = &mut *std::ptr::addr_of_mut!((*shape_table_mut()));

        for round in 1..=2 {
            if round == 2 || *opt_vars().p_guicursor == 0 {
                clear_shape_table();
                if *opt_vars().p_guicursor == 0 {
                    ui_mode_info_set();
                    return std::ptr::null();
                }
            }
            let mut modep: *const c_char = opt_vars().p_guicursor;
            while !modep.is_null() && *modep != 0 {
                let colonp = std::ffi::CStr::from_ptr(modep).to_bytes().iter().position(|&b| b == b':').map_or(std::ptr::null(), |i| modep.add(i));
                let commap = std::ffi::CStr::from_ptr(modep).to_bytes().iter().position(|&b| b == b',').map_or(std::ptr::null(), |i| modep.add(i));

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
                                if crate::memory::cstr_prefix_bytes(modep, len).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(entry.name.as_ptr(), len)) {
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
                        let len = if crate::memory::cstr_prefix_bytes(p, 3).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"ver".as_ptr(), 3))
                            || crate::memory::cstr_prefix_bytes(p, 3).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"hor".as_ptr(), 3))
                        {
                            3usize
                        } else if crate::memory::cstr_prefix_bytes(p, 9).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"blinkwait".as_ptr(), 9)) {
                            9usize
                        } else if crate::memory::cstr_prefix_bytes(p, 7).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"blinkon".as_ptr(), 7)) {
                            7usize
                        } else if crate::memory::cstr_prefix_bytes(p, 8).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"blinkoff".as_ptr(), 8)) {
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
                        } else if crate::memory::cstr_prefix_bytes(p, 5).eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"block".as_ptr(), 5)) {
                            if round == 2 {
                                tbl[idx].shape = CursorShape::SHAPE_BLOCK;
                            }
                            p = p.add(5);
                        } else {
                            let mut endp = std::ffi::CStr::from_ptr(p).to_bytes().iter().position(|&b| b == b'-').map_or(std::ptr::null(), |i| p.add(i));
                            if commap.is_null() {
                                if endp.is_null() {
                                    endp = p.add(std::ffi::CStr::from_ptr(p).to_bytes().len());
                                }
                            } else if endp.is_null() || (endp as usize) > (commap as usize) {
                                endp = commap;
                            }
                            let slashp = std::ffi::CStr::from_ptr(p).to_bytes().iter().position(|&b| b == b'/').map_or(std::ptr::null(), |i| p.add(i));
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
#[cfg(test)]
pub fn cursor_mode_str2int(mode: *const c_char) -> c_int {
    let mode_bytes = unsafe { std::ffi::CStr::from_ptr(mode).to_bytes() };
    let tbl = unsafe { &*std::ptr::addr_of!((*shape_table_mut())) };
    for (mode_idx, entry) in tbl.iter().enumerate() {
        if entry.full_name.to_bytes() == mode_bytes {
            return mode_idx as c_int;
        }
    }
    logmsg!(
            LOGLVL_WRN,
            std::ptr::null(),
            c"cursor_mode_str2int".as_ptr(),
            284,
            true,
            c"Unknown mode %s".as_ptr(),
            mode,
        );
    -1
}

/// Check if a syntax id is used as a cursor style (`cursor_mode_uses_syn_id`).
pub fn cursor_mode_uses_syn_id(syn_id: c_int) -> bool {
    unsafe {
        if *opt_vars().p_guicursor == 0 {
            return false;
        }
        let tbl = &*std::ptr::addr_of!((*shape_table_mut()));
        for entry in tbl.iter() {
            if entry.id == syn_id || entry.id_lm == syn_id {
                return true;
            }
        }
        false
    }
}

/// Return the index into `(*shape_table_mut())[]` for the current mode (`cursor_get_mode_idx`).
pub fn cursor_get_mode_idx() -> c_int {
    unsafe {
        if g_st().State == MODE_SHOWMATCH {
            SHAPE_IDX_SM as c_int
        } else if g_st().State == MODE_TERMINAL {
            SHAPE_IDX_TERM as c_int
        } else if (g_st().State & VREPLACE_FLAG) != 0 || (g_st().State & REPLACE_FLAG) != 0 {
            SHAPE_IDX_R as c_int
        } else if (g_st().State & MODE_INSERT) != 0 {
            SHAPE_IDX_I as c_int
        } else if (g_st().State & MODE_CMDLINE) != 0 {
            if cmdline_at_end() {
                SHAPE_IDX_C as c_int
            } else if cmdline_overstrike() {
                SHAPE_IDX_CR as c_int
            } else {
                SHAPE_IDX_CI as c_int
            }
        } else if g_st().finish_op {
            SHAPE_IDX_O as c_int
        } else if g_st().Visual.active {
            if *opt_vars().p_sel as u8 == b'e' {
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
