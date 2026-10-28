//! JSON and MessagePack decoding into Vimscript `TypVal` values (`src/nvim/eval/decode.c`).

use crate::charset::vim_str2nr;
use crate::eval::encode::encode_list_write;
use crate::eval::string2float;
use crate::eval::typval::{
    TvList, TypVal, TypvalVval, VAR_UNLOCKED, VarType, tv_blob_alloc_ret, tv_clear, tv_dict_add,
    tv_dict_alloc, tv_dict_find, tv_dict_item_alloc, tv_dict_item_alloc_len, tv_list_alloc,
    tv_list_append_list, tv_list_append_number, tv_list_append_owned_tv, tv_list_len, tv_list_ref,
};
use crate::eval::vars::eval_msgpack_type_lists_mut;
use crate::hashtab::hashitem_empty;
use crate::mbyte::{utf_char2bytes, utf_char2len, utf_ptr2char, utf_ptr2len};
use crate::message::{emsg, semsg};
use crate::mpack::object::MpackData;
use std::ffi::c_char;

pub(crate) const K_LIST_LEN_MAY_KNOW: isize = -3;

pub(crate) const K_MP_INTEGER: usize = 2;
pub(crate) const K_MP_MAP: usize = 6;
pub(crate) const K_MP_EXT: usize = 7;

pub(crate) const SURROGATE_HI_START: i32 = 0xD800;
pub(crate) const SURROGATE_HI_END: i32 = 0xDBFF;
pub(crate) const SURROGATE_LO_START: i32 = 0xDC00;
pub(crate) const SURROGATE_LO_END: i32 = 0xDFFF;
pub(crate) const SURROGATE_FIRST_CHAR: i32 = 0x10000;

use crate::mpack::mpack_core::{MPACK_OK, MpackTokenType};
use crate::mpack::object::{
    MpackNode, MpackParser, mpack_parent_node, mpack_parse, mpack_parser_init,
};

use crate::api::private::converter::{K_BOOL_VAR_FALSE, K_BOOL_VAR_TRUE, K_SPECIAL_VAR_NULL};
use crate::mpack::conv::{
    mpack_unpack_boolean, mpack_unpack_float_fast, mpack_unpack_sint, mpack_unpack_uint,
};

#[derive(Clone, Copy)]
struct JsonContainerStackItem {
    stack_index: usize,
    special_val: *mut TvList,
    s: *const c_char,
    container: TypVal,
}

#[derive(Clone, Copy)]
struct ValuesStackItem {
    is_special_string: bool,
    didcomma: bool,
    didcolon: bool,
    val: TypVal,
}

fn create_special_dict(rettv: &mut TypVal, mp_type: usize, val: TypVal) {
    let dict = tv_dict_alloc();
    let type_di = tv_dict_item_alloc_len(c"_TYPE", 5);
    type_di.di_tv.v_type = VarType::VarList;
    type_di.di_tv.v_lock = VAR_UNLOCKED;
    type_di.di_tv.vval =
        crate::eval::typval::TypvalVval::List((*eval_msgpack_type_lists_mut())[mp_type].cast_mut());
    tv_list_ref(unsafe { type_di.di_tv.vval.v_list().as_mut() });
    tv_dict_add(&mut *dict, type_di);

    let val_di = tv_dict_item_alloc_len(c"_VAL", 4);
    val_di.di_tv = val;
    tv_dict_add(&mut *dict, val_di);
    dict.dv_refcount += 1;
    *rettv = TypVal {
        v_type: VarType::VarDict,
        v_lock: VAR_UNLOCKED,
        vval: TypvalVval::Dict(dict),
    };
}

/// Create a special dictionary representing a MessagePack MAP (`decode_create_map_special_dict`).
pub fn decode_create_map_special_dict(ret_tv: &mut TypVal, len: isize) -> &'static mut TvList {
    let list = tv_list_alloc(len);
    tv_list_ref(Some(&mut *list));
    create_special_dict(
        ret_tv,
        K_MP_MAP,
        TypVal {
            v_type: VarType::VarList,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::List(list),
        },
    );
    list
}

/// Convert `s[..len]` into a `VAR_STRING` or `VarBlob` `TypVal` (`decode_string`).
pub fn decode_string(
    s: impl crate::strings::IntoCCharPtr,
    len: usize,
    force_blob: bool,
    _s_allocated: bool,
) -> TypVal {
    let s = s.into_c_char_ptr();
    assert!(!s.is_null() || len == 0);
    let has_nul = if !s.is_null() && len > 0 {
        let slice = crate::memory::cstr_slice(s, len);
        slice.contains(&0)
    } else {
        false
    };
    let use_blob = force_blob || has_nul;

    if use_blob {
        let mut tv = TypVal {
            v_type: VarType::VarUnknown,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        {
            let b = &mut *tv_blob_alloc_ret(&mut tv);
            if !s.is_null() && len > 0 {
                b.bv_ga.extend_from_slice(crate::memory::cstr_slice(s, len));
            }
        }
        return tv;
    }

    let _ = use_blob;
    let mut tv = TypVal {
        v_type: VarType::VarString,
        v_lock: VAR_UNLOCKED,
        vval: TypvalVval::String(std::ptr::null_mut()),
    };
    if !s.is_null() {
        crate::eval::typval::tv_set_string_len(&mut tv, s, len);
    }
    tv
}

fn json_decoder_pop(
    obj: ValuesStackItem,
    stack: &mut Vec<ValuesStackItem>,
    container_stack: &mut Vec<JsonContainerStackItem>,
    pp: &mut *const c_char,
    next_map_special: &mut bool,
    didcomma: &mut bool,
    didcolon: &mut bool,
) -> bool {
    if container_stack.is_empty() {
        stack.push(obj);
        return true;
    }
    let mut last_container = *container_stack.last().unwrap();
    let mut val_location = *pp;
    if obj.val.v_type == last_container.container.v_type
        && std::ptr::eq(
            obj.val.vval.v_list(),
            last_container.container.vval.v_list(),
        )
    {
        container_stack.pop();
        val_location = last_container.s;
        last_container = *container_stack.last().unwrap();
    }

    {
        if last_container.container.v_type == VarType::VarList {
            if tv_list_len(unsafe { last_container.container.vval.v_list().as_ref() }) != 0
                && !obj.didcomma
            {
                semsg!(c"E474: Expected comma before list item: %s", val_location,);
                let mut v = obj.val;
                tv_clear(&mut v);
                return false;
            }
            assert!(last_container.special_val.is_null());
            tv_list_append_owned_tv(
                unsafe { &mut *last_container.container.vval.v_list() },
                obj.val,
            );
        } else if last_container.stack_index + 2 == stack.len() {
            if !obj.didcolon {
                semsg!(
                    c"E474: Expected colon before dictionary value: %s",
                    val_location,
                );
                let mut v = obj.val;
                tv_clear(&mut v);
                return false;
            }
            let mut key = stack.pop().unwrap();
            if last_container.special_val.is_null() {
                assert!(!(key.is_special_string || key.val.vval.v_string().is_null()));
                let obj_di = tv_dict_item_alloc(key.val.vval.v_string());
                tv_clear(&mut key.val);
                let dict = last_container.container.vval.v_dict();
                if !tv_dict_add(unsafe { &mut *dict }, obj_di) {
                    std::process::abort();
                }
                obj_di.di_tv = obj.val;
            } else {
                let kv_pair = tv_list_alloc(2);
                tv_list_append_list(unsafe { &mut *last_container.special_val }, kv_pair);
                tv_list_append_owned_tv(&mut *kv_pair, key.val);
                tv_list_append_owned_tv(&mut *kv_pair, obj.val);
            }
        } else {
            if !obj.is_special_string && obj.val.v_type != VarType::VarString {
                semsg!(c"E474: Expected string key: %s", *pp);
                let mut v = obj.val;
                tv_clear(&mut v);
                return false;
            } else if !obj.didcomma && last_container.special_val.is_null() {
                let dict = last_container.container.vval.v_dict();
                if unsafe { (*dict).dv_hashtab.ht_used } != 0 {
                    semsg!(
                        c"E474: Expected comma before dictionary key: %s",
                        val_location,
                    );
                    let mut v = obj.val;
                    tv_clear(&mut v);
                    return false;
                }
            }
            let dict = last_container.container.vval.v_dict();
            if last_container.special_val.is_null()
                && (obj.is_special_string
                    || obj.val.vval.v_string().is_null()
                    || !tv_dict_find(unsafe { dict.as_ref() }, obj.val.vval.v_string(), -1)
                        .is_null())
            {
                let mut v = obj.val;
                tv_clear(&mut v);
                container_stack.pop();
                let last_container_val = stack[last_container.stack_index];
                while stack.len() > last_container.stack_index {
                    let mut popped = stack.pop().unwrap();
                    tv_clear(&mut popped.val);
                }
                *pp = last_container.s;
                *didcomma = last_container_val.didcomma;
                *didcolon = last_container_val.didcolon;
                *next_map_special = true;
                return true;
            }
            stack.push(obj);
        }
    }
    true
}

fn parse_json_string(
    buf: impl crate::strings::IntoCCharPtr,
    buf_len: usize,
    pp: &mut *const c_char,
    stack: &mut Vec<ValuesStackItem>,
    container_stack: &mut Vec<JsonContainerStackItem>,
    next_map_special: &mut bool,
    didcomma: &mut bool,
    didcolon: &mut bool,
) -> bool {
    let buf = buf.into_c_char_ptr();
    let e = buf.wrapping_add(buf_len);
    let mut p = (*pp).wrapping_add(1);
    let s = p;
    let mut len = 0usize;

    {
        while p < e && crate::memory::cstr_first_byte(p) != b'"' {
            if crate::memory::cstr_first_byte(p) == b'\\' {
                p = p.wrapping_add(1);
                if p == e {
                    semsg!(
                        c"E474: Unfinished escape sequence: %.*s",
                        buf_len as i32,
                        buf,
                    );
                    *pp = p;
                    return false;
                }
                match crate::memory::cstr_first_byte(p) {
                    b'u' => {
                        if p.wrapping_add(4) >= e {
                            semsg!(
                                c"E474: Unfinished unicode escape sequence: %.*s",
                                buf_len as i32,
                                buf,
                            );
                            *pp = p;
                            return false;
                        }
                        if !((crate::memory::cstr_byte_at(p, 1)).is_ascii_hexdigit()
                            && (crate::memory::cstr_byte_at(p, 2)).is_ascii_hexdigit()
                            && (crate::memory::cstr_byte_at(p, 3)).is_ascii_hexdigit()
                            && (crate::memory::cstr_byte_at(p, 4)).is_ascii_hexdigit())
                        {
                            let start = p.wrapping_sub(1);
                            semsg!(
                                c"E474: Expected four hex digits after \\u: %.*s",
                                ((e as usize) - (start as usize)) as i32,
                                start,
                            );
                            *pp = p;
                            return false;
                        }
                        len += 3;
                        p = p.wrapping_add(5);
                    }
                    b'\\' | b'/' | b'"' | b't' | b'b' | b'n' | b'r' | b'f' => {
                        len += 1;
                        p = p.wrapping_add(1);
                    }
                    _ => {
                        let start = p.wrapping_sub(1);
                        semsg!(
                            c"E474: Unknown escape sequence: %.*s",
                            ((e as usize) - (start as usize)) as i32,
                            start,
                        );
                        *pp = p;
                        return false;
                    }
                }
            } else {
                let p_byte = crate::memory::cstr_first_byte(p);
                if p_byte < 0x20 {
                    semsg!(
                        c"E474: ASCII control characters cannot be present inside string: %.*s",
                        ((e as usize) - (p as usize)) as i32,
                        p,
                    );
                    *pp = p;
                    return false;
                }
                let ch = utf_ptr2char(p);
                if ch >= 0x80
                    && i32::from(p_byte) == ch
                    && !(ch == 0xC3
                        && p.wrapping_add(1) < e
                        && (crate::memory::cstr_byte_at(p, 1)) == 0x83)
                {
                    semsg!(
                        c"E474: Only UTF-8 strings allowed: %.*s",
                        ((e as usize) - (p as usize)) as i32,
                        p,
                    );
                    *pp = p;
                    return false;
                } else if ch > 0x10FFFF {
                    semsg!(
                        c"E474: Only UTF-8 code points up to U+10FFFF are allowed to appear unescaped: %.*s",
                        ((e as usize) - (p as usize)) as i32,
                        p,
                    );
                    *pp = p;
                    return false;
                }
                let ch_len = utf_char2len(ch) as usize;
                debug_assert_eq!(ch_len, if ch != 0 { utf_ptr2len(p) as usize } else { 1 });
                len += ch_len;
                p = p.wrapping_add(ch_len);
            }
        }

        if p == e || crate::memory::cstr_first_byte(p) != b'"' {
            semsg!(c"E474: Expected string end: %.*s", buf_len as i32, buf,);
            *pp = p;
            return false;
        }

        let mut str_buf_vec: Vec<c_char> = vec![0; len + 1];
        let str_buf = str_buf_vec.as_mut_ptr();
        let mut fst_in_pair: i32 = 0;
        let mut str_end = str_buf;
        let mut t = s;

        while t < p {
            if (crate::memory::cstr_first_byte(t) != b'\\'
                || crate::memory::cstr_byte_at(t, 1) != b'u')
                && fst_in_pair != 0
            {
                str_end = str_end.wrapping_add(utf_char2bytes(fst_in_pair, str_end) as usize);
                fst_in_pair = 0;
            }
            if crate::memory::cstr_first_byte(t) == b'\\' {
                t = t.wrapping_add(1);
                match crate::memory::cstr_first_byte(t) {
                    b'u' => {
                        let ubuf = [
                            crate::memory::cstr_byte_at(t, 1),
                            crate::memory::cstr_byte_at(t, 2),
                            crate::memory::cstr_byte_at(t, 3),
                            crate::memory::cstr_byte_at(t, 4),
                        ];
                        t = t.wrapping_add(4);
                        let hex_str = unsafe { std::str::from_utf8_unchecked(&ubuf) };
                        let ch = i32::from(u16::from_str_radix(hex_str, 16).unwrap_or(0));
                        if (SURROGATE_HI_START..=SURROGATE_HI_END).contains(&ch) {
                            if fst_in_pair != 0 {
                                str_end = str_end
                                    .wrapping_add(utf_char2bytes(fst_in_pair, str_end) as usize);
                            }
                            fst_in_pair = ch;
                        } else if (SURROGATE_LO_START..=SURROGATE_LO_END).contains(&ch)
                            && fst_in_pair != 0
                        {
                            let full_char = (ch - SURROGATE_LO_START)
                                + ((fst_in_pair - SURROGATE_HI_START) << 10)
                                + SURROGATE_FIRST_CHAR;
                            str_end =
                                str_end.wrapping_add(utf_char2bytes(full_char, str_end) as usize);
                            fst_in_pair = 0;
                        } else {
                            if fst_in_pair != 0 {
                                str_end = str_end
                                    .wrapping_add(utf_char2bytes(fst_in_pair, str_end) as usize);
                                fst_in_pair = 0;
                            }
                            str_end = str_end.wrapping_add(utf_char2bytes(ch, str_end) as usize);
                        }
                    }
                    b'\\' => {
                        unsafe {
                            *str_end = b'\\' as c_char;
                        }
                        str_end = str_end.wrapping_add(1);
                    }
                    b'/' => {
                        unsafe {
                            *str_end = b'/' as c_char;
                        }
                        str_end = str_end.wrapping_add(1);
                    }
                    b'"' => {
                        unsafe {
                            *str_end = b'"' as c_char;
                        }
                        str_end = str_end.wrapping_add(1);
                    }
                    b't' => {
                        unsafe {
                            *str_end = b'\t' as c_char;
                        }
                        str_end = str_end.wrapping_add(1);
                    }
                    b'b' => {
                        unsafe {
                            *str_end = 0x08 as c_char;
                        }
                        str_end = str_end.wrapping_add(1);
                    }
                    b'n' => {
                        unsafe {
                            *str_end = b'\n' as c_char;
                        }
                        str_end = str_end.wrapping_add(1);
                    }
                    b'r' => {
                        unsafe {
                            *str_end = b'\r' as c_char;
                        }
                        str_end = str_end.wrapping_add(1);
                    }
                    b'f' => {
                        unsafe {
                            *str_end = 0x0c as c_char;
                        }
                        str_end = str_end.wrapping_add(1);
                    }
                    _ => std::process::abort(),
                }
            } else {
                unsafe {
                    *str_end = *t;
                }
                str_end = str_end.wrapping_add(1);
            }
            t = t.wrapping_add(1);
        }

        if fst_in_pair != 0 {
            str_end = str_end.wrapping_add(utf_char2bytes(fst_in_pair, str_end) as usize);
        }
        unsafe {
            *str_end = 0;
        }

        let out_len = (str_end as usize) - (str_buf as usize);
        let obj = decode_string(str_buf, out_len, false, false);
        let is_sp = obj.v_type != VarType::VarString;
        let pop_res = json_decoder_pop(
            ValuesStackItem {
                is_special_string: is_sp,
                didcomma: *didcomma,
                didcolon: *didcolon,
                val: obj,
            },
            stack,
            container_stack,
            &mut p,
            next_map_special,
            didcomma,
            didcolon,
        );
        *pp = p;
        pop_res
    }
}

fn parse_json_number(
    buf: impl crate::strings::IntoCCharPtr,
    buf_len: usize,
    pp: &mut *const c_char,
    stack: &mut Vec<ValuesStackItem>,
    container_stack: &mut Vec<JsonContainerStackItem>,
    next_map_special: &mut bool,
    didcomma: &mut bool,
    didcolon: &mut bool,
) -> bool {
    let buf = buf.into_c_char_ptr();
    let e = buf.wrapping_add(buf_len);
    let mut p = *pp;
    let s = p;
    let mut fracs: *const c_char = std::ptr::null();
    let mut exps: *const c_char = std::ptr::null();
    let mut exps_s: *const c_char = std::ptr::null();

    {
        if crate::memory::cstr_first_byte(p) == b'-' {
            p = p.wrapping_add(1);
        }
        let ints = p;
        if p < e {
            while p < e && (crate::memory::cstr_first_byte(p)).is_ascii_digit() {
                p = p.wrapping_add(1);
            }
            if p != ints.wrapping_add(1) && crate::memory::cstr_first_byte(ints) == b'0' {
                semsg!(
                    c"E474: Leading zeroes are not allowed: %.*s",
                    ((e as usize) - (s as usize)) as i32,
                    s,
                );
                *pp = p;
                return false;
            }
            if p < e && p != ints {
                if crate::memory::cstr_first_byte(p) == b'.' {
                    p = p.wrapping_add(1);
                    fracs = p;
                    while p < e && (crate::memory::cstr_first_byte(p)).is_ascii_digit() {
                        p = p.wrapping_add(1);
                    }
                }
                if p < e
                    && (fracs.is_null() || p != fracs)
                    && matches!(crate::memory::cstr_first_byte(p), b'e' | b'E')
                {
                    p = p.wrapping_add(1);
                    exps_s = p;
                    if p < e && matches!(crate::memory::cstr_first_byte(p), b'-' | b'+') {
                        p = p.wrapping_add(1);
                    }
                    exps = p;
                    while p < e && (crate::memory::cstr_first_byte(p)).is_ascii_digit() {
                        p = p.wrapping_add(1);
                    }
                }
            }
        }

        if p == ints {
            semsg!(
                c"E474: Missing number after minus sign: %.*s",
                ((e as usize) - (s as usize)) as i32,
                s,
            );
            *pp = p;
            return false;
        } else if p == fracs || (!fracs.is_null() && exps_s == fracs.wrapping_add(1)) {
            semsg!(
                c"E474: Missing number after decimal dot: %.*s",
                ((e as usize) - (s as usize)) as i32,
                s,
            );
            *pp = p;
            return false;
        } else if p == exps {
            semsg!(
                c"E474: Missing exponent: %.*s",
                ((e as usize) - (s as usize)) as i32,
                s,
            );
            *pp = p;
            return false;
        }

        let mut tv = TypVal {
            v_type: VarType::VarNumber,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        let exp_num_len = (p as usize) - (s as usize);
        if !fracs.is_null() || !exps.is_null() {
            let mut flt: f64 = 0.0;
            let num_len = string2float(s, &mut flt);
            if exp_num_len != num_len {
                semsg!(
                    c"E685: internal error: while converting number \"%.*s\" to float string2float consumed %zu bytes in place of %zu",
                    exp_num_len as i32,
                    s,
                    num_len,
                    exp_num_len,
                );
            }
            tv.v_type = VarType::VarFloat;
            tv.vval = TypvalVval::Float(flt);
        } else {
            let mut nr: i64 = 0;
            let mut num_len: i32 = 0;
            vim_str2nr(
                s,
                None,
                Some(&mut num_len),
                0,
                Some(&mut nr),
                None,
                exp_num_len as i32,
                true,
                None,
            );
            if (exp_num_len as i32) != num_len {
                semsg!(
                    c"E685: internal error: while converting number \"%.*s\" to integer vim_str2nr consumed %i bytes in place of %zu",
                    exp_num_len as i32,
                    s,
                    num_len,
                    exp_num_len,
                );
            }
            tv.vval = TypvalVval::Number(nr);
        }

        if !json_decoder_pop(
            ValuesStackItem {
                is_special_string: false,
                didcomma: *didcomma,
                didcolon: *didcolon,
                val: tv,
            },
            stack,
            container_stack,
            &mut p,
            next_map_special,
            didcomma,
            didcolon,
        ) {
            *pp = p;
            return false;
        }
        if !*next_map_special {
            p = p.wrapping_sub(1);
        }
        *pp = p;
        true
    }
}

/// Convert JSON string `buf[..buf_len]` into a Vimscript `TypVal` (`json_decode_string`).
pub fn json_decode_string(
    buf: impl crate::strings::IntoCCharPtr,
    buf_len: usize,
    rettv: &mut TypVal,
) -> bool {
    let buf = buf.into_c_char_ptr();
    unsafe {
        let mut p = buf;
        let e = buf.wrapping_add(buf_len);
        while p < e
            && matches!(
                crate::memory::cstr_first_byte(p),
                b' ' | b'\t' | b'\n' | b'\r'
            )
        {
            p = p.wrapping_add(1);
        }
        if p == e {
            emsg(c"E474: Attempt to decode a blank string");
            return false;
        }

        let mut stack: Vec<ValuesStackItem> = Vec::new();
        let mut container_stack: Vec<JsonContainerStackItem> = Vec::new();
        rettv.v_type = VarType::VarUnknown;
        let mut didcomma = false;
        let mut didcolon = false;
        let mut next_map_special = false;

        let fail_cleanup = |mut stack: Vec<ValuesStackItem>| -> bool {
            while let Some(mut item) = stack.pop() {
                tv_clear(&mut item.val);
            }
            false
        };

        'outer: while p < e {
            loop {
                debug_assert!(crate::memory::cstr_first_byte(p) == b'{' || !next_map_special);
                let len_rem = ((e as usize) - (p as usize)) as i32;
                match crate::memory::cstr_first_byte(p) {
                    b'}' | b']' => {
                        if container_stack.is_empty() {
                            semsg!(c"E474: No container to close: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        let last_container = *container_stack.last().unwrap();
                        if crate::memory::cstr_first_byte(p) == b'}'
                            && last_container.container.v_type != VarType::VarDict
                        {
                            semsg!(c"E474: Closing list with curly bracket: %.*s", len_rem, p,);
                            return fail_cleanup(stack);
                        } else if crate::memory::cstr_first_byte(p) == b']'
                            && last_container.container.v_type != VarType::VarList
                        {
                            semsg!(
                                c"E474: Closing dictionary with square bracket: %.*s",
                                len_rem,
                                p,
                            );
                            return fail_cleanup(stack);
                        } else if didcomma {
                            semsg!(c"E474: Trailing comma: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        } else if didcolon {
                            semsg!(c"E474: Expected value after colon: %.*s", len_rem, p,);
                            return fail_cleanup(stack);
                        } else if last_container.stack_index + 1 != stack.len() {
                            semsg!(c"E474: Expected value: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        if stack.len() == 1 {
                            p = p.wrapping_add(1);
                            container_stack.pop();
                            break 'outer;
                        } else {
                            let popped = stack.pop().unwrap();
                            if !json_decoder_pop(
                                popped,
                                &mut stack,
                                &mut container_stack,
                                &mut p,
                                &mut next_map_special,
                                &mut didcomma,
                                &mut didcolon,
                            ) {
                                return fail_cleanup(stack);
                            }
                            debug_assert!(!next_map_special);
                        }
                    }
                    b',' => {
                        if container_stack.is_empty() {
                            semsg!(c"E474: Comma not inside container: %.*s", len_rem, p,);
                            return fail_cleanup(stack);
                        }
                        let last_container = *container_stack.last().unwrap();
                        let is_empty_cont = if last_container.special_val.is_null() {
                            if last_container.container.v_type == VarType::VarDict {
                                let d = last_container.container.vval.v_dict();
                                (*d).dv_hashtab.ht_used == 0
                            } else {
                                tv_list_len(last_container.container.vval.v_list().as_ref()) == 0
                            }
                        } else {
                            tv_list_len(last_container.special_val.as_ref()) == 0
                        };
                        if didcomma {
                            semsg!(c"E474: Duplicate comma: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        } else if didcolon {
                            semsg!(c"E474: Comma after colon: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        } else if last_container.container.v_type == VarType::VarDict
                            && last_container.stack_index + 1 != stack.len()
                        {
                            semsg!(c"E474: Using comma in place of colon: %.*s", len_rem, p,);
                            return fail_cleanup(stack);
                        } else if is_empty_cont {
                            semsg!(c"E474: Leading comma: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        didcomma = true;
                        p = p.wrapping_add(1);
                        continue 'outer;
                    }
                    b':' => {
                        if container_stack.is_empty() {
                            semsg!(c"E474: Colon not inside container: %.*s", len_rem, p,);
                            return fail_cleanup(stack);
                        }
                        let last_container = *container_stack.last().unwrap();
                        if last_container.container.v_type != VarType::VarDict {
                            semsg!(c"E474: Using colon not in dictionary: %.*s", len_rem, p,);
                            return fail_cleanup(stack);
                        } else if last_container.stack_index + 2 != stack.len() {
                            semsg!(c"E474: Unexpected colon: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        } else if didcomma {
                            semsg!(c"E474: Colon after comma: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        } else if didcolon {
                            semsg!(c"E474: Duplicate colon: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        didcolon = true;
                        p = p.wrapping_add(1);
                        continue 'outer;
                    }
                    b' ' | b'\t' | b'\n' | b'\r' => {
                        p = p.wrapping_add(1);
                        continue 'outer;
                    }
                    b'n' => {
                        if p.wrapping_add(3) >= e
                            || std::slice::from_raw_parts(p.wrapping_add(1).cast::<u8>(), 3)
                                != b"ull"
                        {
                            semsg!(c"E474: Expected null: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        p = p.wrapping_add(3);
                        let tv = TypVal {
                            v_type: VarType::VarSpecial,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval::Number(K_SPECIAL_VAR_NULL),
                        };
                        if !json_decoder_pop(
                            ValuesStackItem {
                                is_special_string: false,
                                didcomma,
                                didcolon,
                                val: tv,
                            },
                            &mut stack,
                            &mut container_stack,
                            &mut p,
                            &mut next_map_special,
                            &mut didcomma,
                            &mut didcolon,
                        ) {
                            return fail_cleanup(stack);
                        }
                        if next_map_special {
                            continue;
                        }
                    }
                    b't' => {
                        if p.wrapping_add(3) >= e
                            || std::slice::from_raw_parts(p.wrapping_add(1).cast::<u8>(), 3)
                                != b"rue"
                        {
                            semsg!(c"E474: Expected true: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        p = p.wrapping_add(3);
                        let tv = TypVal {
                            v_type: VarType::VarBool,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval::Number(K_BOOL_VAR_TRUE),
                        };
                        if !json_decoder_pop(
                            ValuesStackItem {
                                is_special_string: false,
                                didcomma,
                                didcolon,
                                val: tv,
                            },
                            &mut stack,
                            &mut container_stack,
                            &mut p,
                            &mut next_map_special,
                            &mut didcomma,
                            &mut didcolon,
                        ) {
                            return fail_cleanup(stack);
                        }
                        if next_map_special {
                            continue;
                        }
                    }
                    b'f' => {
                        if p.wrapping_add(4) >= e
                            || std::slice::from_raw_parts(p.wrapping_add(1).cast::<u8>(), 4)
                                != b"alse"
                        {
                            semsg!(c"E474: Expected false: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        p = p.wrapping_add(4);
                        let tv = TypVal {
                            v_type: VarType::VarBool,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval::Number(K_BOOL_VAR_FALSE),
                        };
                        if !json_decoder_pop(
                            ValuesStackItem {
                                is_special_string: false,
                                didcomma,
                                didcolon,
                                val: tv,
                            },
                            &mut stack,
                            &mut container_stack,
                            &mut p,
                            &mut next_map_special,
                            &mut didcomma,
                            &mut didcolon,
                        ) {
                            return fail_cleanup(stack);
                        }
                        if next_map_special {
                            continue;
                        }
                    }
                    b'"' => {
                        if !parse_json_string(
                            buf,
                            buf_len,
                            &mut p,
                            &mut stack,
                            &mut container_stack,
                            &mut next_map_special,
                            &mut didcomma,
                            &mut didcolon,
                        ) {
                            return fail_cleanup(stack);
                        }
                        if next_map_special {
                            continue;
                        }
                    }
                    b'-' | b'0'..=b'9' => {
                        if !parse_json_number(
                            buf,
                            buf_len,
                            &mut p,
                            &mut stack,
                            &mut container_stack,
                            &mut next_map_special,
                            &mut didcomma,
                            &mut didcolon,
                        ) {
                            return fail_cleanup(stack);
                        }
                        if next_map_special {
                            continue;
                        }
                    }
                    b'[' => {
                        let list = tv_list_alloc(K_LIST_LEN_MAY_KNOW);
                        tv_list_ref(Some(&mut *list));
                        let tv = TypVal {
                            v_type: VarType::VarList,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval::List(list),
                        };
                        container_stack.push(JsonContainerStackItem {
                            stack_index: stack.len(),
                            s: p,
                            container: tv,
                            special_val: std::ptr::null_mut(),
                        });
                        stack.push(ValuesStackItem {
                            is_special_string: false,
                            didcomma,
                            didcolon,
                            val: tv,
                        });
                    }
                    b'{' => {
                        let mut tv = TypVal {
                            v_type: VarType::VarUnknown,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval::Number(0),
                        };
                        let mut val_list: *mut TvList = std::ptr::null_mut();
                        if next_map_special {
                            next_map_special = false;
                            val_list = decode_create_map_special_dict(&mut tv, K_LIST_LEN_MAY_KNOW);
                        } else {
                            let dict = tv_dict_alloc();
                            dict.dv_refcount += 1;
                            tv = TypVal {
                                v_type: VarType::VarDict,
                                v_lock: VAR_UNLOCKED,
                                vval: TypvalVval::Dict(dict),
                            };
                        }
                        container_stack.push(JsonContainerStackItem {
                            stack_index: stack.len(),
                            s: p,
                            container: tv,
                            special_val: val_list,
                        });
                        stack.push(ValuesStackItem {
                            is_special_string: false,
                            didcomma,
                            didcolon,
                            val: tv,
                        });
                    }
                    _ => {
                        semsg!(c"E474: Unidentified byte: %.*s", len_rem, p);
                        return fail_cleanup(stack);
                    }
                }
                break;
            }
            didcomma = false;
            didcolon = false;
            if container_stack.is_empty() {
                p = p.wrapping_add(1);
                break 'outer;
            }
            p = p.wrapping_add(1);
        }

        while p < e {
            match crate::memory::cstr_first_byte(p) {
                b'\n' | b' ' | b'\t' | b'\r' => p = p.wrapping_add(1),
                _ => {
                    let len_rem = ((e as usize) - (p as usize)) as i32;
                    semsg!(c"E474: Trailing characters: %.*s", len_rem, p);
                    return fail_cleanup(stack);
                }
            }
        }

        if stack.len() == 1 && container_stack.is_empty() {
            *rettv = stack.pop().unwrap().val;
            return true;
        }
        semsg!(c"E474: Unexpected end of input: %.*s", buf_len as i32, buf,);
        fail_cleanup(stack)
    }
}

fn positive_integer_to_special_typval(rettv: &mut TypVal, val: u64) {
    if val <= (i64::MAX as u64) {
        *rettv = TypVal {
            v_type: VarType::VarNumber,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(val as i64),
        };
    } else {
        let list = tv_list_alloc(4);
        tv_list_ref(Some(&mut *list));
        create_special_dict(
            rettv,
            K_MP_INTEGER,
            TypVal {
                v_type: VarType::VarList,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::List(list),
            },
        );
        tv_list_append_number(&mut *list, 1);
        tv_list_append_number(&mut *list, ((val >> 62) & 0x3) as i64);
        tv_list_append_number(&mut *list, ((val >> 31) & 0x7FFF_FFFF) as i64);
        tv_list_append_number(&mut *list, (val & 0x7FFF_FFFF) as i64);
    }
}

fn typval_parse_enter(parser: &mut MpackParser, node: &mut MpackNode) {
    unsafe {
        let parser = &mut *parser;
        let node = &mut *node;
        let mut result: *mut TypVal = std::ptr::null_mut();
        let parent = mpack_parent_node(&raw mut *node);
        if !parent.is_null() {
            match (*parent).tok.r#type {
                MpackTokenType::MpackTokenArray => {
                    let list = (*parent).data[1].list();
                    result = tv_list_append_owned_tv(
                        &mut *list,
                        TypVal {
                            v_type: VarType::VarUnknown,
                            v_lock: 0,
                            vval: TypvalVval::Number(0),
                        },
                    );
                }
                MpackTokenType::MpackTokenMap => {
                    let pos = (*parent).pos;
                    let key_visited = (*parent).key_visited as usize;
                    let items = (*parent).data[1].map_pairs_mut();
                    result = &raw mut items[pos][key_visited];
                }
                MpackTokenType::MpackTokenStr
                | MpackTokenType::MpackTokenBin
                | MpackTokenType::MpackTokenExt => {
                    assert_eq!(node.tok.r#type, MpackTokenType::MpackTokenChunk);
                }
                _ => std::process::abort(),
            }
        } else {
            result = parser.data.typval();
        }

        node.data[0] = MpackData::Typval(result);
        node.data[1] = MpackData::Null;

        match node.tok.r#type {
            MpackTokenType::MpackTokenNil => {
                *result = TypVal {
                    v_type: VarType::VarSpecial,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(K_SPECIAL_VAR_NULL),
                };
            }
            MpackTokenType::MpackTokenBoolean => {
                let b = mpack_unpack_boolean(node.tok);
                *result = TypVal {
                    v_type: VarType::VarBool,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(if b { K_BOOL_VAR_TRUE } else { K_BOOL_VAR_FALSE }),
                };
            }
            MpackTokenType::MpackTokenSint => {
                *result = TypVal {
                    v_type: VarType::VarNumber,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(mpack_unpack_sint(node.tok)),
                };
            }
            MpackTokenType::MpackTokenUint => {
                positive_integer_to_special_typval(&mut *result, mpack_unpack_uint(node.tok));
            }
            MpackTokenType::MpackTokenFloat => {
                *result = TypVal {
                    v_type: VarType::VarFloat,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Float(mpack_unpack_float_fast(node.tok)),
                };
            }
            MpackTokenType::MpackTokenBin
            | MpackTokenType::MpackTokenStr
            | MpackTokenType::MpackTokenExt => {
                node.data[1] = MpackData::OwnedBytes(vec![0u8; node.tok.length as usize]);
            }
            MpackTokenType::MpackTokenChunk => {
                let data = (*parent).data[1].owned_bytes_mut();
                let len = node.tok.length as usize;
                if len > 0 {
                    let chunk = crate::memory::cstr_slice(node.tok.data.chunk_ptr(), len);
                    data[(*parent).pos..((*parent).pos + len)].copy_from_slice(chunk);
                }
            }
            MpackTokenType::MpackTokenArray => {
                let list = tv_list_alloc(node.tok.length as isize);
                tv_list_ref(Some(&mut *list));
                *result = TypVal {
                    v_type: VarType::VarList,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::List(list),
                };
                node.data[1] = MpackData::List(list);
            }
            MpackTokenType::MpackTokenMap => {
                node.data[1] =
                    MpackData::MapPairs(vec![[TypVal::new(); 2]; node.tok.length as usize]);
            }
            MpackTokenType::MpackTokenNone => std::process::abort(),
        }
    }
}

/// Free un-exited nodes on `parser` after a parse error (`typval_parser_error_free`).
pub fn typval_parser_error_free(p: &mut MpackParser) {
    for i in 1..=(p.size as usize) {
        let node = &mut p.items[i];
        match node.tok.r#type {
            MpackTokenType::MpackTokenBin
            | MpackTokenType::MpackTokenStr
            | MpackTokenType::MpackTokenExt
            | MpackTokenType::MpackTokenMap => {
                node.data[1] = MpackData::Null;
            }
            _ => {}
        }
    }
}

fn typval_parse_exit(_parser: &mut MpackParser, node: &mut MpackNode) {
    let node = &mut *node;
    match node.tok.r#type {
        MpackTokenType::MpackTokenBin | MpackTokenType::MpackTokenStr => {
            let result = node.data[0].typval();
            let bytes = node.data[1].take_owned_bytes();
            *result = decode_string(
                bytes.as_ptr().cast::<c_char>(),
                node.tok.length as usize,
                false,
                false,
            );
        }
        MpackTokenType::MpackTokenExt => {
            let result = node.data[0].typval();
            let bytes = node.data[1].take_owned_bytes();
            let list = tv_list_alloc(2);
            tv_list_ref(Some(&mut *list));
            tv_list_append_number(&mut *list, i64::from(node.tok.data.ext_type()));
            let ext_val_list = tv_list_alloc(K_LIST_LEN_MAY_KNOW);
            tv_list_append_list(&mut *list, ext_val_list);
            create_special_dict(
                &mut *result,
                K_MP_EXT,
                TypVal {
                    v_type: VarType::VarList,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::List(list),
                },
            );
            encode_list_write(
                Some(&mut *ext_val_list),
                bytes.as_ptr().cast::<c_char>(),
                node.tok.length as usize,
            );
        }
        MpackTokenType::MpackTokenMap => {
            let result = node.data[0].typval();
            let mut items = node.data[1].take_map_pairs();
            let len = node.tok.length as usize;
            let mut can_be_dict = items[..len].iter().all(|pair| {
                let key = &pair[0];
                key.v_type == VarType::VarString
                    && !key.vval.v_string().is_null()
                    && unsafe { *key.vval.v_string() } != 0
            });

            if can_be_dict {
                let dict = tv_dict_alloc();
                dict.dv_refcount += 1;
                *result = TypVal {
                    v_type: VarType::VarDict,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Dict(dict),
                };
                for pair in &items[..len] {
                    let key = pair[0].vval.v_string();
                    let keylen = crate::memory::cstr_len(key);
                    let di = crate::eval::typval::tv_dict_item_alloc_len(key, keylen);
                    if !tv_dict_add(&mut *dict, di) {
                        let mut todo = dict.dv_hashtab.ht_used;
                        let mut hi = dict.dv_hashtab.array_ptr();
                        while todo > 0 {
                            if !hashitem_empty(unsafe { &*hi }) {
                                todo -= 1;
                                let d = crate::eval::typval::tv_dict_hi2di(hi);
                                d.di_tv.v_type = VarType::VarSpecial;
                                d.di_tv.vval =
                                    crate::eval::typval::TypvalVval::Number(K_SPECIAL_VAR_NULL);
                            }
                            hi = hi.wrapping_add(1);
                        }
                        tv_clear(&mut *result);
                        crate::eval::typval::tv_dict_item_free(di);
                        can_be_dict = false;
                        break;
                    }
                    di.di_tv = pair[1];
                }
            }

            if can_be_dict {
                for pair in &mut items[..len] {
                    tv_clear(&mut pair[0]);
                }
            } else {
                let list = decode_create_map_special_dict(&mut *result, len as isize);
                for pair in &items[..len] {
                    let kv_pair = tv_list_alloc(2);
                    tv_list_append_list(&mut *list, kv_pair);
                    tv_list_append_owned_tv(&mut *kv_pair, pair[0]);
                    tv_list_append_owned_tv(&mut *kv_pair, pair[1]);
                }
            }
        }
        _ => {}
    }
}

/// Parse msgpack data into a `TypVal` using `parser` (`mpack_parse_typval`).
pub fn mpack_parse_typval(
    parser: &mut MpackParser,
    data: &mut *const c_char,
    size: &mut usize,
) -> i32 {
    mpack_parse(
        parser,
        data,
        size,
        Some(typval_parse_enter),
        Some(typval_parse_exit),
    )
}

/// Unpack msgpack buffer `*data[..*size]` into `*ret` (`unpack_typval`).
pub fn unpack_typval(data: &mut *const c_char, size: &mut usize, ret: &mut TypVal) -> i32 {
    ret.v_type = VarType::VarUnknown;
    let mut parser = MpackParser::new(0);
    mpack_parser_init(&mut parser, 0);
    parser.data = MpackData::Typval(ret);
    let status = mpack_parse_typval(&mut parser, data, size);
    if status != MPACK_OK {
        typval_parser_error_free(&mut parser);
        tv_clear(&mut *ret);
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_string_copies_ascii() {
        let mut tv = decode_string(c"hello", 5, false, false);
        assert_eq!(tv.v_type, VarType::VarString);
        assert_eq!(
            crate::memory::cstr_from_ptr(tv.vval.v_string())
                .to_str()
                .unwrap(),
            "hello"
        );
        tv_clear(&mut tv);
    }

    #[test]
    fn json_decode_string_respects_length_and_rejects_malformed_inputs() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::globals::g_st().emsg_skip += 1;
        let mut rettv = TypVal {
            v_type: VarType::VarUnknown,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };

        for &(s, len) in &[
            (b"null" as &[u8], 1usize),
            (b"true", 1),
            (b"false", 1),
            (b"null", 2),
            (b"true", 2),
            (b"false", 2),
            (b"null", 3),
            (b"true", 3),
            (b"false", 3),
            (b"false", 4),
            (b"\"t\"", 2),
            (b"\"\"", 1),
            (b"]test", 1),
            (b"[}test", 2),
            (b"{]test", 2),
            (b"[1,]test", 4),
            (b"{\"1\":}test", 6),
            (b"{\"1\"}test", 5),
            (b",test", 1),
            (b"[1,,1]test", 6),
            (b"{\"1\":,}test", 7),
            (b"{\"1\",}test", 6),
            (b"{,}test", 3),
            (b"[,]test", 3),
            (b":test", 1),
            (b"[:]test", 3),
            (b"{:}test", 3),
            (b"{\"1\"::1}test", 8),
            (b"ntest", 1),
            (b"ttest", 1),
            (b"ftest", 1),
            (b"\"\\test", 2),
            (b"\"\\u\"test", 4),
            (b"\"\\uXXXX\"est", 8),
            (b"\"\\?\"test", 4),
            (b"\"\t\"test", 3),
            (b"\"\xc2\"test", 3),
            (b"\"\xfc\x90\x80\x80\x80\x80\"test", 8),
            (b"\"test", 1),
            (b"-test", 1),
            (b"-0", 1),
            (b"-1.test", 3),
            (b"-1.0etest", 5),
            (b"?test", 1),
            (b"1?test", 2),
            (b"[1test", 2),
        ] {
            assert!(!json_decode_string(s.as_ptr().cast(), len, &mut rettv));
            assert_eq!(rettv.v_type, VarType::VarUnknown);
        }

        assert!(json_decode_string(c"true", 4, &mut rettv));
        assert_eq!(rettv.v_type, VarType::VarBool);
        assert_eq!(rettv.vval.v_bool(), K_BOOL_VAR_TRUE as i32);
        tv_clear(&mut rettv);

        crate::globals::g_st().emsg_skip -= 1;
    }
}
