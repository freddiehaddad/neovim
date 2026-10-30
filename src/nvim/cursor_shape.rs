//! Handling of cursor and mouse pointer shapes in various modes (`src/nvim/cursor_shape.c`).

use crate::api::private::helpers::{arena_array, arena_dict, cstr_as_string};
use crate::api::private::validate::{
    Array, Dict, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::ex_getln::{cmdline_at_end, cmdline_overstrike};
use crate::globals::*;
use crate::highlight_group::{syn_check_group, syn_id2attr};
#[cfg(test)]
use crate::log::{LOGLVL_WRN, logmsg};
use crate::memory::Arena;
use crate::option_vars::*;
use crate::ui::ui_mode_info_set;
use std::ffi::CStr;

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
const SHAPE_BOTH: i32 = SHAPE_CURSOR + SHAPE_MOUSE;

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
    pub used_for: i32,
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
    used_for: i32,
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
    entry(c"cmdline_hover", 0, 0, 0, c"e", SHAPE_MOUSE),
    entry(c"statusline_hover", 0, 0, 0, c"s", SHAPE_MOUSE),
    entry(c"statusline_drag", 0, 0, 0, c"sd", SHAPE_MOUSE),
    entry(c"vsep_hover", 0, 0, 0, c"vs", SHAPE_MOUSE),
    entry(c"vsep_drag", 0, 0, 0, c"vd", SHAPE_MOUSE),
    entry(c"more", 0, 0, 0, c"m", SHAPE_MOUSE),
    entry(c"more_lastline", 0, 0, 0, c"ml", SHAPE_MOUSE),
    entry(c"showmatch", 100, 100, 100, c"sm", SHAPE_CURSOR),
    entry(c"terminal", 0, 0, 0, c"t", SHAPE_CURSOR),
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
        let is_cursor = (cur.used_for & SHAPE_CURSOR) != 0;
        let mut dic = arena_dict(arena, 3 + if is_cursor { 9 } else { 0 });
        dict_put_c(
            &mut dic,
            c"name",
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_as_string(cur.full_name)),
            },
        );
        dict_put_c(
            &mut dic,
            c"short_name",
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_as_string(cur.name)),
            },
        );
        if (cur.used_for & SHAPE_MOUSE) != 0 {
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
                    data: ObjectData::String(cstr_as_string(shape_cstr)),
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
pub fn parse_shape_opt(what: i32) -> Option<&'static CStr> {
    let mut idx: usize = 0;
    let mut found_ve = false;
    let tbl = unsafe { &mut *std::ptr::addr_of_mut!((*shape_table_mut())) };
    let guicursor_ptr = opt_vars().p_guicursor;
    let guicursor = crate::memory::cstr_as_bytes(guicursor_ptr);

    for round in 1..=2 {
        if round == 2 || guicursor.is_empty() {
            clear_shape_table();
            if guicursor.is_empty() {
                ui_mode_info_set();
                return None;
            }
        }
        let mut mode_idx = 0usize;
        while mode_idx < guicursor.len() {
            let rest = &guicursor[mode_idx..];
            let colon_rel = rest.iter().position(|&b| b == b':');
            let comma_rel = rest.iter().position(|&b| b == b',');
            let Some(colon_rel) = colon_rel.filter(|&col| comma_rel.is_none_or(|com| col < com))
            else {
                return Some(c"E545: Missing colon");
            };
            if colon_rel == 0 {
                return Some(c"E546: Illegal mode");
            }
            let colon_idx = mode_idx + colon_rel;
            let comma_idx = comma_rel.map_or(guicursor.len(), |c| mode_idx + c);

            let mut all_idx: isize = -1;
            let mut p_idx = colon_idx + 1;
            while mode_idx < colon_idx || all_idx >= 0 {
                if all_idx < 0 {
                    let m0 = guicursor[mode_idx];
                    let m1 = guicursor.get(mode_idx + 1).copied().unwrap_or(0);
                    let len = if m1 == b'-' || m1 == b':' {
                        1usize
                    } else {
                        2usize
                    };

                    if len == 1 && m0.eq_ignore_ascii_case(&b'a') {
                        all_idx = (SHAPE_IDX_COUNT - 1) as isize;
                    } else {
                        let mode_slice = &guicursor[mode_idx..mode_idx + len];
                        idx = SHAPE_IDX_COUNT;
                        for (i, entry) in tbl.iter().enumerate() {
                            if entry.name.to_bytes().eq_ignore_ascii_case(mode_slice) {
                                idx = i;
                                break;
                            }
                        }
                        if idx == SHAPE_IDX_COUNT || (tbl[idx].used_for & what) == 0 {
                            return Some(c"E546: Illegal mode");
                        }
                        if len == 2 && m0 == b'v' && m1 == b'e' {
                            found_ve = true;
                        }
                    }
                    mode_idx += len + 1;
                }

                if all_idx >= 0 {
                    idx = all_idx as usize;
                    all_idx -= 1;
                }

                p_idx = colon_idx + 1;
                while p_idx < comma_idx {
                    let rem = &guicursor[p_idx..comma_idx];
                    let first_byte = rem[0];
                    let len = if rem.get(..3).is_some_and(|s| {
                        s.eq_ignore_ascii_case(b"ver") || s.eq_ignore_ascii_case(b"hor")
                    }) {
                        3usize
                    } else if rem
                        .get(..9)
                        .is_some_and(|s| s.eq_ignore_ascii_case(b"blinkwait"))
                    {
                        9usize
                    } else if rem
                        .get(..7)
                        .is_some_and(|s| s.eq_ignore_ascii_case(b"blinkon"))
                    {
                        7usize
                    } else if rem
                        .get(..8)
                        .is_some_and(|s| s.eq_ignore_ascii_case(b"blinkoff"))
                    {
                        8usize
                    } else {
                        0usize
                    };

                    if len != 0 {
                        p_idx += len;
                        if !guicursor.get(p_idx).is_some_and(u8::is_ascii_digit) {
                            return Some(c"E548: Digit expected");
                        }
                        let mut n: i64 = 0;
                        while let Some(&d) = guicursor.get(p_idx).filter(|b| b.is_ascii_digit()) {
                            n = n.saturating_mul(10).saturating_add(i64::from(d - b'0'));
                            p_idx += 1;
                        }
                        let n = n.min(i64::from(i32::MAX)) as i32;
                        if len == 3 {
                            if n == 0 {
                                return Some(c"E549: Illegal percentage");
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
                    } else if rem
                        .get(..5)
                        .is_some_and(|s| s.eq_ignore_ascii_case(b"block"))
                    {
                        if round == 2 {
                            tbl[idx].shape = CursorShape::ShapeBlock;
                        }
                        p_idx += 5;
                    } else {
                        let end_rel = rem.iter().position(|&b| b == b'-').unwrap_or(rem.len());
                        let end_idx = p_idx + end_rel;
                        let slash_idx = rem[..end_rel]
                            .iter()
                            .position(|&b| b == b'/')
                            .map(|s| p_idx + s);
                        let mut saved_hl_id = first_byte as i32;
                        if let Some(s_idx) = slash_idx {
                            saved_hl_id =
                                syn_check_group(guicursor_ptr.wrapping_add(p_idx), s_idx - p_idx);
                            p_idx = s_idx + 1;
                        }
                        if round == 2 {
                            tbl[idx].id =
                                syn_check_group(guicursor_ptr.wrapping_add(p_idx), end_idx - p_idx);
                            tbl[idx].id_lm = tbl[idx].id;
                            if slash_idx.is_some() {
                                tbl[idx].id = saved_hl_id;
                            }
                        }
                        p_idx = end_idx;
                    }

                    if guicursor.get(p_idx).copied() == Some(b'-') {
                        p_idx += 1;
                    }
                }
            }
            mode_idx = p_idx;
            if guicursor.get(mode_idx).copied() == Some(b',') {
                mode_idx += 1;
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
    None
}

/// Map cursor mode from string to integer (`cursor_mode_str2int`).
#[cfg(test)]
pub fn cursor_mode_str2int(mode: &std::ffi::CStr) -> i32 {
    let mode_bytes = mode.to_bytes();
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
