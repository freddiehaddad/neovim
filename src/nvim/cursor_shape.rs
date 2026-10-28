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
use std::ffi::{CStr, c_char};

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

pub const SHAPE_MOUSE: i32 = 1;
pub const SHAPE_CURSOR: i32 = 2;
const SHAPE_BOTH: c_char = (SHAPE_CURSOR + SHAPE_MOUSE) as c_char;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    ShapeBlock = 0,
    ShapeHor = 1,
    ShapeVer = 2,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Cursorentry {
    pub full_name: &'static CStr,
    pub shape: CursorShape,
    pub mshape: i32,
    pub percentage: i32,
    pub blinkwait: i32,
    pub blinkon: i32,
    pub blinkoff: i32,
    pub id: i32,
    pub id_lm: i32,
    pub name: &'static CStr,
    pub used_for: c_char,
}

use crate::state::{
    MODE_CMDLINE, MODE_INSERT, MODE_SHOWMATCH, MODE_TERMINAL, REPLACE_FLAG, VREPLACE_FLAG,
};

const fn entry(
    full_name: &'static CStr,
    blinkwait: i32,
    blinkon: i32,
    blinkoff: i32,
    name: &'static CStr,
    used_for: c_char,
) -> Cursorentry {
    Cursorentry {
        full_name,
        shape: CursorShape::ShapeBlock,
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

static SHAPE_TABLE: GlobalState<[Cursorentry; SHAPE_IDX_COUNT]> = GlobalState::new([
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
pub fn shape_table_mut() -> &'static mut [Cursorentry; SHAPE_IDX_COUNT] {
    unsafe { &mut *SHAPE_TABLE.as_ptr() }
}

#[inline]
fn dict_put_c(dic: &mut Dict, key: &'static CStr, value: Object) {
    assert!(dic.size < dic.capacity);
    let key_bytes = key.to_bytes();
    {
        *dic.next_slot_mut() = KeyValuePair {
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
        entry.shape = CursorShape::ShapeBlock;
        entry.blinkwait = 0;
        entry.blinkon = 0;
        entry.blinkoff = 0;
        entry.id = 0;
        entry.id_lm = 0;
    }
}

/// Converts `(*shape_table_mut())` into an `Array` of `Dict`s (`mode_style_array`).
pub fn mode_style_array(arena: &mut Arena) -> Array {
    let mut all = arena_array(arena, SHAPE_IDX_COUNT);
    let tbl = unsafe { &*std::ptr::addr_of!((*shape_table_mut())) };

    for cur in tbl.iter() {
        let is_cursor = (cur.used_for as i32 & SHAPE_CURSOR) != 0;
        let mut dic = arena_dict(arena, 3 + if is_cursor { 9 } else { 0 });
        dict_put_c(
            &mut dic,
            c"name",
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_as_string(cur.full_name.as_ptr())),
            },
        );
        dict_put_c(
            &mut dic,
            c"short_name",
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_as_string(cur.name.as_ptr())),
            },
        );
        if (cur.used_for as i32 & SHAPE_MOUSE) != 0 {
            dict_put_c(
                &mut dic,
                c"mouse_shape",
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(cur.mshape as i64),
                },
            );
        }
        if is_cursor {
            let shape_cstr = match cur.shape {
                CursorShape::ShapeBlock => c"block",
                CursorShape::ShapeVer => c"vertical",
                CursorShape::ShapeHor => c"horizontal",
            };
            dict_put_c(
                &mut dic,
                c"cursor_shape",
                Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(cstr_as_string(shape_cstr.as_ptr())),
                },
            );
            dict_put_c(
                &mut dic,
                c"cell_percentage",
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(cur.percentage as i64),
                },
            );
            dict_put_c(
                &mut dic,
                c"blinkwait",
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(cur.blinkwait as i64),
                },
            );
            dict_put_c(
                &mut dic,
                c"blinkon",
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(cur.blinkon as i64),
                },
            );
            dict_put_c(
                &mut dic,
                c"blinkoff",
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(cur.blinkoff as i64),
                },
            );
            dict_put_c(
                &mut dic,
                c"hl_id",
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(cur.id as i64),
                },
            );
            dict_put_c(
                &mut dic,
                c"id_lm",
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(cur.id_lm as i64),
                },
            );
            dict_put_c(
                &mut dic,
                c"attr_id",
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
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
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(if cur.id_lm != 0 {
                        syn_id2attr(cur.id_lm) as i64
                    } else {
                        0
                    }),
                },
            );
        }
        assert!(all.size < all.capacity);
        *all.next_slot_mut() = Object {
            obj_type: ObjectType::ObjectTypeDict,
            data: ObjectData::Dict(dic),
        };
        all.size += 1;
    }
    all
}

/// Parses the `'guicursor'` option (`parse_shape_opt`).
pub fn parse_shape_opt(what: i32) -> *const c_char {
    let mut p: *const c_char = std::ptr::null();
    let mut idx: usize = 0;
    let mut found_ve = false;
    let tbl = unsafe { &mut *std::ptr::addr_of_mut!((*shape_table_mut())) };

    for round in 1..=2 {
        if round == 2 || crate::memory::cstr_is_empty(opt_vars().p_guicursor) {
            clear_shape_table();
            if crate::memory::cstr_is_empty(opt_vars().p_guicursor) {
                ui_mode_info_set();
                return std::ptr::null();
            }
        }
        let mut modep: *const c_char = opt_vars().p_guicursor;
        while !modep.is_null() && unsafe { *modep } != 0 {
            let colonp = crate::memory::cstr_as_bytes(modep)
                .iter()
                .position(|&b| b == b':')
                .map_or(std::ptr::null(), |i| modep.wrapping_add(i));
            let commap = crate::memory::cstr_as_bytes(modep)
                .iter()
                .position(|&b| b == b',')
                .map_or(std::ptr::null(), |i| modep.wrapping_add(i));

            if colonp.is_null() || (!commap.is_null() && (commap as usize) < (colonp as usize)) {
                return c"E545: Missing colon".as_ptr();
            }
            if colonp == modep {
                return c"E546: Illegal mode".as_ptr();
            }

            let mut all_idx: isize = -1;
            while (modep as usize) < (colonp as usize) || all_idx >= 0 {
                if all_idx < 0 {
                    let m1 = crate::memory::cstr_byte_at(modep, 1);
                    let len = if m1 == b'-' || m1 == b':' {
                        1usize
                    } else {
                        2usize
                    };

                    if len == 1
                        && (crate::memory::cstr_first_byte(modep)).eq_ignore_ascii_case(&b'a')
                    {
                        all_idx = (SHAPE_IDX_COUNT - 1) as isize;
                    } else {
                        idx = SHAPE_IDX_COUNT;
                        for (i, entry) in tbl.iter().enumerate() {
                            if crate::memory::cstr_prefix_bytes(modep, len).eq_ignore_ascii_case(
                                crate::memory::cstr_prefix_bytes(entry.name.as_ptr(), len),
                            ) {
                                idx = i;
                                break;
                            }
                        }
                        if idx == SHAPE_IDX_COUNT || (tbl[idx].used_for as i32 & what) == 0 {
                            return c"E546: Illegal mode".as_ptr();
                        }
                        if len == 2 && crate::memory::cstr_first_byte(modep) == b'v' && m1 == b'e' {
                            found_ve = true;
                        }
                    }
                    modep = modep.wrapping_add(len + 1);
                }

                if all_idx >= 0 {
                    idx = all_idx as usize;
                    all_idx -= 1;
                }

                p = colonp.wrapping_add(1);
                while unsafe { *p } != 0 && crate::memory::cstr_first_byte(p) != b',' {
                    let first_byte = crate::memory::cstr_first_byte(p);
                    let len = if crate::memory::cstr_prefix_bytes(p, 3).eq_ignore_ascii_case(b"ver")
                        || crate::memory::cstr_prefix_bytes(p, 3).eq_ignore_ascii_case(b"hor")
                    {
                        3usize
                    } else if crate::memory::cstr_prefix_bytes(p, 9)
                        .eq_ignore_ascii_case(b"blinkwait")
                    {
                        9usize
                    } else if crate::memory::cstr_prefix_bytes(p, 7)
                        .eq_ignore_ascii_case(b"blinkon")
                    {
                        7usize
                    } else if crate::memory::cstr_prefix_bytes(p, 8)
                        .eq_ignore_ascii_case(b"blinkoff")
                    {
                        8usize
                    } else {
                        0usize
                    };

                    if len != 0 {
                        p = p.wrapping_add(len);
                        if !(crate::memory::cstr_first_byte(p)).is_ascii_digit() {
                            return c"E548: Digit expected".as_ptr();
                        }
                        let n = getdigits_int((&raw mut p).cast(), false, 0);
                        if len == 3 {
                            if n == 0 {
                                return c"E549: Illegal percentage".as_ptr();
                            }
                            if round == 2 {
                                if first_byte.eq_ignore_ascii_case(&b'v') {
                                    tbl[idx].shape = CursorShape::ShapeVer;
                                } else {
                                    tbl[idx].shape = CursorShape::ShapeHor;
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
                    } else if crate::memory::cstr_prefix_bytes(p, 5).eq_ignore_ascii_case(b"block")
                    {
                        if round == 2 {
                            tbl[idx].shape = CursorShape::ShapeBlock;
                        }
                        p = p.wrapping_add(5);
                    } else {
                        let mut endp = crate::memory::cstr_as_bytes(p)
                            .iter()
                            .position(|&b| b == b'-')
                            .map_or(std::ptr::null(), |i| p.wrapping_add(i));
                        if commap.is_null() {
                            if endp.is_null() {
                                endp = p.wrapping_add(crate::memory::cstr_len(p));
                            }
                        } else if endp.is_null() || (endp as usize) > (commap as usize) {
                            endp = commap;
                        }
                        let slashp = crate::memory::cstr_as_bytes(p)
                            .iter()
                            .position(|&b| b == b'/')
                            .map_or(std::ptr::null(), |i| p.wrapping_add(i));
                        let mut saved_hl_id = first_byte as i32;
                        if !slashp.is_null() && (slashp as usize) < (endp as usize) {
                            saved_hl_id = syn_check_group(p, (slashp as usize) - (p as usize));
                            p = slashp.wrapping_add(1);
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

                    if crate::memory::cstr_first_byte(p) == b'-' {
                        p = p.wrapping_add(1);
                    }
                }
            }
            modep = p;
            if !modep.is_null() && crate::memory::cstr_first_byte(modep) == b',' {
                modep = modep.wrapping_add(1);
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

/// Map cursor mode from string to integer (`cursor_mode_str2int`).
#[cfg(test)]
pub fn cursor_mode_str2int(mode: &std::ffi::CStr) -> i32 {
    let mode_bytes = { mode.to_bytes() };
    let tbl = unsafe { &*std::ptr::addr_of!((*shape_table_mut())) };
    for (mode_idx, entry) in tbl.iter().enumerate() {
        if entry.full_name.to_bytes() == mode_bytes {
            return mode_idx as i32;
        }
    }
    logmsg!(
        LOGLVL_WRN,
        std::ptr::null(),
        c"cursor_mode_str2int",
        284,
        true,
        c"Unknown mode %s",
        mode.as_ptr(),
    );
    -1
}

/// Check if a syntax id is used as a cursor style (`cursor_mode_uses_syn_id`).
pub fn cursor_mode_uses_syn_id(syn_id: i32) -> bool {
    if crate::memory::cstr_is_empty(opt_vars().p_guicursor) {
        return false;
    }
    let tbl = unsafe { &*std::ptr::addr_of!((*shape_table_mut())) };
    for entry in tbl.iter() {
        if entry.id == syn_id || entry.id_lm == syn_id {
            return true;
        }
    }
    false
}

/// Return the index into `(*shape_table_mut())[]` for the current mode (`cursor_get_mode_idx`).
pub fn cursor_get_mode_idx() -> i32 {
    if g_st().state_global == MODE_SHOWMATCH {
        SHAPE_IDX_SM as i32
    } else if g_st().state_global == MODE_TERMINAL {
        SHAPE_IDX_TERM as i32
    } else if (g_st().state_global & VREPLACE_FLAG) != 0
        || (g_st().state_global & REPLACE_FLAG) != 0
    {
        SHAPE_IDX_R as i32
    } else if (g_st().state_global & MODE_INSERT) != 0 {
        SHAPE_IDX_I as i32
    } else if (g_st().state_global & MODE_CMDLINE) != 0 {
        if cmdline_at_end() {
            SHAPE_IDX_C as i32
        } else if cmdline_overstrike() {
            SHAPE_IDX_CR as i32
        } else {
            SHAPE_IDX_CI as i32
        }
    } else if g_st().finish_op {
        SHAPE_IDX_O as i32
    } else if g_st().visual_global.active {
        if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
            SHAPE_IDX_VE as i32
        } else {
            SHAPE_IDX_V as i32
        }
    } else {
        SHAPE_IDX_N as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_mode_str2int_finds_modes() {
        assert_eq!(cursor_mode_str2int(c"normal"), SHAPE_IDX_N as i32);
        assert_eq!(cursor_mode_str2int(c"terminal"), SHAPE_IDX_TERM as i32);
        assert_eq!(cursor_mode_str2int(c"nonexistent"), -1);
    }
}
