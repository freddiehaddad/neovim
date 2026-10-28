//! JSON and MessagePack decoding into Vimscript `typval_T` values (`src/nvim/eval/decode.c`).

use crate::charset::vim_str2nr;
use crate::eval::encode::encode_list_write;
use crate::eval::string2float;
use crate::eval::typval::{
    TypvalVval, VAR_UNLOCKED, VarType, list_T, tv_blob_alloc_ret, tv_clear, tv_dict_add,
    tv_dict_alloc, tv_dict_find, tv_dict_item_alloc, tv_dict_item_alloc_len, tv_list_alloc,
    tv_list_append_list, tv_list_append_number, tv_list_append_owned_tv, tv_list_len, tv_list_ref,
    typval_T,
};
use crate::eval::vars::eval_msgpack_type_lists_mut;
use crate::hashtab::hashitem_empty;
use crate::mbyte::{utf_char2bytes, utf_char2len, utf_ptr2char, utf_ptr2len};
use crate::message::{emsg, semsg};
use crate::mpack::object::mpack_data_t;
use std::ffi::{c_char, c_int};

pub(crate) const K_LIST_LEN_MAY_KNOW: isize = -3;

pub(crate) const K_MP_INTEGER: usize = 2;
pub(crate) const K_MP_MAP: usize = 6;
pub(crate) const K_MP_EXT: usize = 7;

pub(crate) const SURROGATE_HI_START: c_int = 0xD800;
pub(crate) const SURROGATE_HI_END: c_int = 0xDBFF;
pub(crate) const SURROGATE_LO_START: c_int = 0xDC00;
pub(crate) const SURROGATE_LO_END: c_int = 0xDFFF;
pub(crate) const SURROGATE_FIRST_CHAR: c_int = 0x10000;

use crate::mpack::mpack_core::{MPACK_OK, mpack_token_type_t};
use crate::mpack::object::{
    mpack_node_t, mpack_parent_node, mpack_parse, mpack_parser_init, mpack_parser_t,
};

use crate::api::private::converter::{K_BOOL_VAR_FALSE, K_BOOL_VAR_TRUE, K_SPECIAL_VAR_NULL};
use crate::mpack::conv::{
    mpack_unpack_boolean, mpack_unpack_float_fast, mpack_unpack_sint, mpack_unpack_uint,
};

#[derive(Clone, Copy)]
struct JsonContainerStackItem {
    stack_index: usize,
    special_val: *mut list_T,
    s: *const c_char,
    container: typval_T,
}

#[derive(Clone, Copy)]
struct ValuesStackItem {
    is_special_string: bool,
    didcomma: bool,
    didcolon: bool,
    val: typval_T,
}

fn create_special_dict(rettv: &mut typval_T, mp_type: usize, val: typval_T) {
    unsafe {
        let dict = tv_dict_alloc();
        let type_di = tv_dict_item_alloc_len(c"_TYPE", 5);
        (*type_di).di_tv.v_type = VarType::VAR_LIST;
        (*type_di).di_tv.v_lock = VAR_UNLOCKED;
        (*type_di).di_tv.vval = crate::eval::typval::TypvalVval::List((*eval_msgpack_type_lists_mut())[mp_type].cast_mut());
        tv_list_ref((*type_di).di_tv.vval.v_list());
        tv_dict_add(&mut *dict, type_di);

        let val_di = tv_dict_item_alloc_len(c"_VAL", 4);
        (*val_di).di_tv = val;
        tv_dict_add(&mut *dict, val_di);
        (*dict).dv_refcount += 1;
        *rettv = typval_T {
            v_type: VarType::VAR_DICT,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Dict(dict),
        };
    }
}

/// Create a special dictionary representing a MessagePack MAP (`decode_create_map_special_dict`).
pub fn decode_create_map_special_dict(ret_tv: &mut typval_T, len: isize) -> *mut list_T {
    unsafe {
        let list = tv_list_alloc(len);
        tv_list_ref(list);
        create_special_dict(
            ret_tv,
            K_MP_MAP,
            typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::List(list),
            },
        );
        list
    }
}

/// Convert `s[..len]` into a `VAR_STRING` or `VAR_BLOB` `typval_T` (`decode_string`).
pub fn decode_string(
    s: impl crate::strings::IntoCCharPtr,
    len: usize,
    force_blob: bool,
    _s_allocated: bool,
) -> typval_T {
    let s = s.into_c_char_ptr();
    assert!(!s.is_null() || len == 0);
    let has_nul = if !s.is_null() && len > 0 {
        let slice = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len) };
        slice.contains(&0)
    } else {
        false
    };
    let use_blob = force_blob || has_nul;

    if use_blob {
        let mut tv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        unsafe {
            let b = &mut *tv_blob_alloc_ret(&mut tv);
            if !s.is_null() && len > 0 {
                b.bv_ga
                    .extend_from_slice(std::slice::from_raw_parts(s.cast::<u8>(), len));
            }
        }
        return tv;
    }

    let _ = use_blob;
    let mut tv = typval_T {
        v_type: VarType::VAR_STRING,
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
        && std::ptr::eq(obj.val.vval.v_list(), last_container.container.vval.v_list())
    {
        container_stack.pop();
        val_location = last_container.s;
        last_container = *container_stack.last().unwrap();
    }

    unsafe {
        if last_container.container.v_type == VarType::VAR_LIST {
            if tv_list_len(last_container.container.vval.v_list()) != 0 && !obj.didcomma {
                semsg!(
                    c"E474: Expected comma before list item: %s",
                    val_location,
                );
                let mut v = obj.val;
                tv_clear(&mut v);
                return false;
            }
            assert!(last_container.special_val.is_null());
            tv_list_append_owned_tv( &mut *last_container.container.vval.v_list(), obj.val);
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
                if !tv_dict_add(&mut *dict, obj_di) {
                    std::process::abort();
                }
                (*obj_di).di_tv = obj.val;
            } else {
                let kv_pair = tv_list_alloc(2);
                tv_list_append_list( &mut *last_container.special_val, kv_pair);
                tv_list_append_owned_tv( &mut *kv_pair, key.val);
                tv_list_append_owned_tv( &mut *kv_pair, obj.val);
            }
        } else {
            if !obj.is_special_string && obj.val.v_type != VarType::VAR_STRING {
                semsg!(c"E474: Expected string key: %s", *pp);
                let mut v = obj.val;
                tv_clear(&mut v);
                return false;
            } else if !obj.didcomma && last_container.special_val.is_null() {
                let dict = last_container.container.vval.v_dict();
                if (*dict).dv_hashtab.ht_used != 0 {
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
                    || !tv_dict_find(dict, obj.val.vval.v_string(), -1).is_null())
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
    buf: *const c_char,
    buf_len: usize,
    pp: &mut *const c_char,
    stack: &mut Vec<ValuesStackItem>,
    container_stack: &mut Vec<JsonContainerStackItem>,
    next_map_special: &mut bool,
    didcomma: &mut bool,
    didcolon: &mut bool,
) -> bool {
    let e = unsafe { buf.add(buf_len) };
    let mut p = unsafe { (*pp).add(1) };
    let s = p;
    let mut len = 0usize;

    unsafe {
        while p < e && *p as u8 != b'"' {
            if *p as u8 == b'\\' {
                p = p.add(1);
                if p == e {
                    semsg!(
                        c"E474: Unfinished escape sequence: %.*s",
                        buf_len as c_int,
                        buf,
                    );
                    *pp = p;
                    return false;
                }
                match *p as u8 {
                    b'u' => {
                        if p.add(4) >= e {
                            semsg!(
                                c"E474: Unfinished unicode escape sequence: %.*s",
                                buf_len as c_int,
                                buf,
                            );
                            *pp = p;
                            return false;
                        }
                        if !((*p.add(1) as u8).is_ascii_hexdigit()
                            && (*p.add(2) as u8).is_ascii_hexdigit()
                            && (*p.add(3) as u8).is_ascii_hexdigit()
                            && (*p.add(4) as u8).is_ascii_hexdigit())
                        {
                            let start = p.sub(1);
                            semsg!(
                                c"E474: Expected four hex digits after \\u: %.*s",
                                ((e as usize) - (start as usize)) as c_int,
                                start,
                            );
                            *pp = p;
                            return false;
                        }
                        len += 3;
                        p = p.add(5);
                    }
                    b'\\' | b'/' | b'"' | b't' | b'b' | b'n' | b'r' | b'f' => {
                        len += 1;
                        p = p.add(1);
                    }
                    _ => {
                        let start = p.sub(1);
                        semsg!(
                            c"E474: Unknown escape sequence: %.*s",
                            ((e as usize) - (start as usize)) as c_int,
                            start,
                        );
                        *pp = p;
                        return false;
                    }
                }
            } else {
                let p_byte = *p as u8;
                if p_byte < 0x20 {
                    semsg!(
                        c"E474: ASCII control characters cannot be present inside string: %.*s",
                        ((e as usize) - (p as usize)) as c_int,
                        p,
                    );
                    *pp = p;
                    return false;
                }
                let ch = utf_ptr2char(p);
                if ch >= 0x80
                    && c_int::from(p_byte) == ch
                    && !(ch == 0xC3 && p.add(1) < e && (*p.add(1) as u8) == 0x83)
                {
                    semsg!(
                        c"E474: Only UTF-8 strings allowed: %.*s",
                        ((e as usize) - (p as usize)) as c_int,
                        p,
                    );
                    *pp = p;
                    return false;
                } else if ch > 0x10FFFF {
                    semsg!(
                        c"E474: Only UTF-8 code points up to U+10FFFF are allowed to appear unescaped: %.*s",
                        ((e as usize) - (p as usize)) as c_int,
                        p,
                    );
                    *pp = p;
                    return false;
                }
                let ch_len = utf_char2len(ch) as usize;
                debug_assert_eq!(ch_len, if ch != 0 { utf_ptr2len(p) as usize } else { 1 });
                len += ch_len;
                p = p.add(ch_len);
            }
        }

        if p == e || *p as u8 != b'"' {
            semsg!(
                c"E474: Expected string end: %.*s",
                buf_len as c_int,
                buf,
            );
            *pp = p;
            return false;
        }

        let mut str_buf_vec: Vec<c_char> = vec![0; len + 1];
        let str_buf = str_buf_vec.as_mut_ptr();
        let mut fst_in_pair: c_int = 0;
        let mut str_end = str_buf;
        let mut t = s;

        while t < p {
            if (*t as u8 != b'\\' || *t.add(1) as u8 != b'u') && fst_in_pair != 0 {
                str_end = str_end.add(utf_char2bytes(fst_in_pair, str_end) as usize);
                fst_in_pair = 0;
            }
            if *t as u8 == b'\\' {
                t = t.add(1);
                match *t as u8 {
                    b'u' => {
                        let ubuf = [
                            *t.add(1) as u8,
                            *t.add(2) as u8,
                            *t.add(3) as u8,
                            *t.add(4) as u8,
                        ];
                        t = t.add(4);
                        let hex_str = std::str::from_utf8_unchecked(&ubuf);
                        let ch = c_int::from(u16::from_str_radix(hex_str, 16).unwrap_or(0));
                        if (SURROGATE_HI_START..=SURROGATE_HI_END).contains(&ch) {
                            if fst_in_pair != 0 {
                                str_end =
                                    str_end.add(utf_char2bytes(fst_in_pair, str_end) as usize);
                            }
                            fst_in_pair = ch;
                        } else if (SURROGATE_LO_START..=SURROGATE_LO_END).contains(&ch)
                            && fst_in_pair != 0
                        {
                            let full_char = (ch - SURROGATE_LO_START)
                                + ((fst_in_pair - SURROGATE_HI_START) << 10)
                                + SURROGATE_FIRST_CHAR;
                            str_end = str_end.add(utf_char2bytes(full_char, str_end) as usize);
                            fst_in_pair = 0;
                        } else {
                            if fst_in_pair != 0 {
                                str_end =
                                    str_end.add(utf_char2bytes(fst_in_pair, str_end) as usize);
                                fst_in_pair = 0;
                            }
                            str_end = str_end.add(utf_char2bytes(ch, str_end) as usize);
                        }
                    }
                    b'\\' => {
                        *str_end = b'\\' as c_char;
                        str_end = str_end.add(1);
                    }
                    b'/' => {
                        *str_end = b'/' as c_char;
                        str_end = str_end.add(1);
                    }
                    b'"' => {
                        *str_end = b'"' as c_char;
                        str_end = str_end.add(1);
                    }
                    b't' => {
                        *str_end = b'\t' as c_char;
                        str_end = str_end.add(1);
                    }
                    b'b' => {
                        *str_end = 0x08 as c_char;
                        str_end = str_end.add(1);
                    }
                    b'n' => {
                        *str_end = b'\n' as c_char;
                        str_end = str_end.add(1);
                    }
                    b'r' => {
                        *str_end = b'\r' as c_char;
                        str_end = str_end.add(1);
                    }
                    b'f' => {
                        *str_end = 0x0c as c_char;
                        str_end = str_end.add(1);
                    }
                    _ => std::process::abort(),
                }
            } else {
                *str_end = *t;
                str_end = str_end.add(1);
            }
            t = t.add(1);
        }

        if fst_in_pair != 0 {
            str_end = str_end.add(utf_char2bytes(fst_in_pair, str_end) as usize);
        }
        *str_end = 0;

        let out_len = (str_end as usize) - (str_buf as usize);
        let obj = decode_string(str_buf, out_len, false, false);
        let is_sp = obj.v_type != VarType::VAR_STRING;
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
    buf: *const c_char,
    buf_len: usize,
    pp: &mut *const c_char,
    stack: &mut Vec<ValuesStackItem>,
    container_stack: &mut Vec<JsonContainerStackItem>,
    next_map_special: &mut bool,
    didcomma: &mut bool,
    didcolon: &mut bool,
) -> bool {
    let e = unsafe { buf.add(buf_len) };
    let mut p = *pp;
    let s = p;
    let mut fracs: *const c_char = std::ptr::null();
    let mut exps: *const c_char = std::ptr::null();
    let mut exps_s: *const c_char = std::ptr::null();

    unsafe {
        if *p as u8 == b'-' {
            p = p.add(1);
        }
        let ints = p;
        if p < e {
            while p < e && (*p as u8).is_ascii_digit() {
                p = p.add(1);
            }
            if p != ints.add(1) && *ints as u8 == b'0' {
                semsg!(
                    c"E474: Leading zeroes are not allowed: %.*s",
                    ((e as usize) - (s as usize)) as c_int,
                    s,
                );
                *pp = p;
                return false;
            }
            if p < e && p != ints {
                if *p as u8 == b'.' {
                    p = p.add(1);
                    fracs = p;
                    while p < e && (*p as u8).is_ascii_digit() {
                        p = p.add(1);
                    }
                }
                if p < e && (fracs.is_null() || p != fracs) && matches!(*p as u8, b'e' | b'E') {
                    p = p.add(1);
                    exps_s = p;
                    if p < e && matches!(*p as u8, b'-' | b'+') {
                        p = p.add(1);
                    }
                    exps = p;
                    while p < e && (*p as u8).is_ascii_digit() {
                        p = p.add(1);
                    }
                }
            }
        }

        if p == ints {
            semsg!(
                c"E474: Missing number after minus sign: %.*s",
                ((e as usize) - (s as usize)) as c_int,
                s,
            );
            *pp = p;
            return false;
        } else if p == fracs || (!fracs.is_null() && exps_s == fracs.add(1)) {
            semsg!(
                c"E474: Missing number after decimal dot: %.*s",
                ((e as usize) - (s as usize)) as c_int,
                s,
            );
            *pp = p;
            return false;
        } else if p == exps {
            semsg!(
                c"E474: Missing exponent: %.*s",
                ((e as usize) - (s as usize)) as c_int,
                s,
            );
            *pp = p;
            return false;
        }

        let mut tv = typval_T {
            v_type: VarType::VAR_NUMBER,
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
                    exp_num_len as c_int,
                    s,
                    num_len,
                    exp_num_len,
                );
            }
            tv.v_type = VarType::VAR_FLOAT;
            tv.vval = TypvalVval::Float(flt);
        } else {
            let mut nr: i64 = 0;
            let mut num_len: c_int = 0;
            vim_str2nr(
                s,
                None,
                Some(&mut num_len),
                0,
                Some(&mut nr),
                None,
                exp_num_len as c_int,
                true,
                None,
            );
            if (exp_num_len as c_int) != num_len {
                semsg!(
                    c"E685: internal error: while converting number \"%.*s\" to integer vim_str2nr consumed %i bytes in place of %zu",
                    exp_num_len as c_int,
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
        )
        {
            *pp = p;
            return false;
        }
        if !*next_map_special {
            p = p.sub(1);
        }
        *pp = p;
        true
    }
}

/// Convert JSON string `buf[..buf_len]` into a Vimscript `typval_T` (`json_decode_string`).
pub fn json_decode_string(
    buf: impl crate::strings::IntoCCharPtr,
    buf_len: usize,
    rettv: &mut typval_T,
) -> bool {
    let buf = buf.into_c_char_ptr();
    unsafe {
        let mut p = buf;
        let e = buf.add(buf_len);
        while p < e && matches!(*p as u8, b' ' | b'\t' | b'\n' | b'\r') {
            p = p.add(1);
        }
        if p == e {
            emsg(c"E474: Attempt to decode a blank string");
            return false;
        }

        let mut stack: Vec<ValuesStackItem> = Vec::new();
        let mut container_stack: Vec<JsonContainerStackItem> = Vec::new();
        rettv.v_type = VarType::VAR_UNKNOWN;
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
                debug_assert!(*p as u8 == b'{' || !next_map_special);
                let len_rem = ((e as usize) - (p as usize)) as c_int;
                match *p as u8 {
                    b'}' | b']' => {
                        if container_stack.is_empty() {
                            semsg!(c"E474: No container to close: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        let last_container = *container_stack.last().unwrap();
                        if *p as u8 == b'}' && last_container.container.v_type != VarType::VAR_DICT
                        {
                            semsg!(
                                c"E474: Closing list with curly bracket: %.*s",
                                len_rem,
                                p,
                            );
                            return fail_cleanup(stack);
                        } else if *p as u8 == b']'
                            && last_container.container.v_type != VarType::VAR_LIST
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
                            semsg!(
                                c"E474: Expected value after colon: %.*s",
                                len_rem,
                                p,
                            );
                            return fail_cleanup(stack);
                        } else if last_container.stack_index + 1 != stack.len() {
                            semsg!(c"E474: Expected value: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        if stack.len() == 1 {
                            p = p.add(1);
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
                            )
                            {
                                return fail_cleanup(stack);
                            }
                            debug_assert!(!next_map_special);
                        }
                    }
                    b',' => {
                        if container_stack.is_empty() {
                            semsg!(
                                c"E474: Comma not inside container: %.*s",
                                len_rem,
                                p,
                            );
                            return fail_cleanup(stack);
                        }
                        let last_container = *container_stack.last().unwrap();
                        let is_empty_cont = if last_container.special_val.is_null() {
                            if last_container.container.v_type == VarType::VAR_DICT {
                                let d = last_container.container.vval.v_dict();
                                (*d).dv_hashtab.ht_used == 0
                            } else {
                                tv_list_len(last_container.container.vval.v_list()) == 0
                            }
                        } else {
                            tv_list_len(last_container.special_val) == 0
                        };
                        if didcomma {
                            semsg!(c"E474: Duplicate comma: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        } else if didcolon {
                            semsg!(c"E474: Comma after colon: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        } else if last_container.container.v_type == VarType::VAR_DICT
                            && last_container.stack_index + 1 != stack.len()
                        {
                            semsg!(
                                c"E474: Using comma in place of colon: %.*s",
                                len_rem,
                                p,
                            );
                            return fail_cleanup(stack);
                        } else if is_empty_cont {
                            semsg!(c"E474: Leading comma: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        didcomma = true;
                        p = p.add(1);
                        continue 'outer;
                    }
                    b':' => {
                        if container_stack.is_empty() {
                            semsg!(
                                c"E474: Colon not inside container: %.*s",
                                len_rem,
                                p,
                            );
                            return fail_cleanup(stack);
                        }
                        let last_container = *container_stack.last().unwrap();
                        if last_container.container.v_type != VarType::VAR_DICT {
                            semsg!(
                                c"E474: Using colon not in dictionary: %.*s",
                                len_rem,
                                p,
                            );
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
                        p = p.add(1);
                        continue 'outer;
                    }
                    b' ' | b'\t' | b'\n' | b'\r' => {
                        p = p.add(1);
                        continue 'outer;
                    }
                    b'n' => {
                        if p.add(3) >= e
                            || std::slice::from_raw_parts(p.add(1).cast::<u8>(), 3) != b"ull"
                        {
                            semsg!(c"E474: Expected null: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        p = p.add(3);
                        let tv = typval_T {
                            v_type: VarType::VAR_SPECIAL,
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
                        )
                        {
                            return fail_cleanup(stack);
                        }
                        if next_map_special {
                            continue;
                        }
                    }
                    b't' => {
                        if p.add(3) >= e
                            || std::slice::from_raw_parts(p.add(1).cast::<u8>(), 3) != b"rue"
                        {
                            semsg!(c"E474: Expected true: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        p = p.add(3);
                        let tv = typval_T {
                            v_type: VarType::VAR_BOOL,
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
                        )
                        {
                            return fail_cleanup(stack);
                        }
                        if next_map_special {
                            continue;
                        }
                    }
                    b'f' => {
                        if p.add(4) >= e
                            || std::slice::from_raw_parts(p.add(1).cast::<u8>(), 4) != b"alse"
                        {
                            semsg!(c"E474: Expected false: %.*s", len_rem, p);
                            return fail_cleanup(stack);
                        }
                        p = p.add(4);
                        let tv = typval_T {
                            v_type: VarType::VAR_BOOL,
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
                        )
                        {
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
                        )
                        {
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
                        )
                        {
                            return fail_cleanup(stack);
                        }
                        if next_map_special {
                            continue;
                        }
                    }
                    b'[' => {
                        let list = tv_list_alloc(K_LIST_LEN_MAY_KNOW);
                        tv_list_ref(list);
                        let tv = typval_T {
                            v_type: VarType::VAR_LIST,
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
                        let mut tv = typval_T {
                            v_type: VarType::VAR_UNKNOWN,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval::Number(0),
                        };
                        let mut val_list: *mut list_T = std::ptr::null_mut();
                        if next_map_special {
                            next_map_special = false;
                            val_list = decode_create_map_special_dict(&mut tv, K_LIST_LEN_MAY_KNOW);
                        } else {
                            let dict = tv_dict_alloc();
                            (*dict).dv_refcount += 1;
                            tv = typval_T {
                                v_type: VarType::VAR_DICT,
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
                p = p.add(1);
                break 'outer;
            }
            p = p.add(1);
        }

        while p < e {
            match *p as u8 {
                b'\n' | b' ' | b'\t' | b'\r' => p = p.add(1),
                _ => {
                    let len_rem = ((e as usize) - (p as usize)) as c_int;
                    semsg!(c"E474: Trailing characters: %.*s", len_rem, p);
                    return fail_cleanup(stack);
                }
            }
        }

        if stack.len() == 1 && container_stack.is_empty() {
            *rettv = stack.pop().unwrap().val;
            return true;
        }
        semsg!(
            c"E474: Unexpected end of input: %.*s",
            buf_len as c_int,
            buf,
        );
        fail_cleanup(stack)
    }
}

fn positive_integer_to_special_typval(rettv: &mut typval_T, val: u64) {
    unsafe {
        if val <= (i64::MAX as u64) {
            *rettv = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(val as i64),
            };
        } else {
            let list = tv_list_alloc(4);
            tv_list_ref(list);
            create_special_dict(
                rettv,
                K_MP_INTEGER,
                typval_T {
                    v_type: VarType::VAR_LIST,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::List(list),
                },
            );
            tv_list_append_number( &mut *list, 1);
            tv_list_append_number( &mut *list, ((val >> 62) & 0x3) as i64);
            tv_list_append_number( &mut *list, ((val >> 31) & 0x7FFF_FFFF) as i64);
            tv_list_append_number( &mut *list, (val & 0x7FFF_FFFF) as i64);
        }
    }
}

fn typval_parse_enter(parser: &mut mpack_parser_t, node: &mut mpack_node_t) {
    unsafe {
        let parser = &mut *parser;
        let node = &mut *node;
        let mut result: *mut typval_T = std::ptr::null_mut();
        let parent = mpack_parent_node(&raw mut *node);
        if !parent.is_null() {
            match (*parent).tok.r#type {
                mpack_token_type_t::MPACK_TOKEN_ARRAY => {
                    let list = (*parent).data[1].list();
                    result = tv_list_append_owned_tv(
                         &mut *list,
                        typval_T {
                            v_type: VarType::VAR_UNKNOWN,
                            v_lock: 0,
                            vval: TypvalVval::Number(0),
                        },
                    );
                }
                mpack_token_type_t::MPACK_TOKEN_MAP => {
                    let pos = (*parent).pos;
                    let key_visited = (*parent).key_visited as usize;
                    let items = (*parent).data[1].map_pairs_mut();
                    result = &raw mut items[pos][key_visited];
                }
                mpack_token_type_t::MPACK_TOKEN_STR
                | mpack_token_type_t::MPACK_TOKEN_BIN
                | mpack_token_type_t::MPACK_TOKEN_EXT => {
                    assert_eq!(node.tok.r#type, mpack_token_type_t::MPACK_TOKEN_CHUNK);
                }
                _ => std::process::abort(),
            }
        } else {
            result = parser.data.typval();
        }

        node.data[0] = mpack_data_t::Typval(result);
        node.data[1] = mpack_data_t::Null;

        match node.tok.r#type {
            mpack_token_type_t::MPACK_TOKEN_NIL => {
                *result = typval_T {
                    v_type: VarType::VAR_SPECIAL,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(K_SPECIAL_VAR_NULL),
                };
            }
            mpack_token_type_t::MPACK_TOKEN_BOOLEAN => {
                let b = mpack_unpack_boolean(node.tok);
                *result = typval_T {
                    v_type: VarType::VAR_BOOL,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(if b { K_BOOL_VAR_TRUE } else { K_BOOL_VAR_FALSE }),
                };
            }
            mpack_token_type_t::MPACK_TOKEN_SINT => {
                *result = typval_T {
                    v_type: VarType::VAR_NUMBER,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(mpack_unpack_sint(node.tok)),
                };
            }
            mpack_token_type_t::MPACK_TOKEN_UINT => {
                positive_integer_to_special_typval(&mut *result, mpack_unpack_uint(node.tok));
            }
            mpack_token_type_t::MPACK_TOKEN_FLOAT => {
                *result = typval_T {
                    v_type: VarType::VAR_FLOAT,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Float(mpack_unpack_float_fast(node.tok)),
                };
            }
            mpack_token_type_t::MPACK_TOKEN_BIN
            | mpack_token_type_t::MPACK_TOKEN_STR
            | mpack_token_type_t::MPACK_TOKEN_EXT => {
                node.data[1] = mpack_data_t::OwnedBytes(vec![0u8; node.tok.length as usize]);
            }
            mpack_token_type_t::MPACK_TOKEN_CHUNK => {
                let data = (*parent).data[1].owned_bytes_mut();
                let len = node.tok.length as usize;
                if len > 0 {
                    let chunk =
                        std::slice::from_raw_parts(node.tok.data.chunk_ptr().cast::<u8>(), len);
                    data[(*parent).pos..((*parent).pos + len)].copy_from_slice(chunk);
                }
            }
            mpack_token_type_t::MPACK_TOKEN_ARRAY => {
                let list = tv_list_alloc(node.tok.length as isize);
                tv_list_ref(list);
                *result = typval_T {
                    v_type: VarType::VAR_LIST,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::List(list),
                };
                node.data[1] = mpack_data_t::List(list);
            }
            mpack_token_type_t::MPACK_TOKEN_MAP => {
                node.data[1] =
                    mpack_data_t::MapPairs(vec![[typval_T::new(); 2]; node.tok.length as usize]);
            }
            mpack_token_type_t::MPACK_TOKEN_NONE => std::process::abort(),
        }
    }
}

/// Free un-exited nodes on `parser` after a parse error (`typval_parser_error_free`).
pub fn typval_parser_error_free(p: &mut mpack_parser_t) {
    for i in 1..=(p.size as usize) {
        let node = &mut p.items[i];
        match node.tok.r#type {
            mpack_token_type_t::MPACK_TOKEN_BIN
            | mpack_token_type_t::MPACK_TOKEN_STR
            | mpack_token_type_t::MPACK_TOKEN_EXT
            | mpack_token_type_t::MPACK_TOKEN_MAP => {
                node.data[1] = mpack_data_t::Null;
            }
            _ => {}
        }
    }
}

fn typval_parse_exit(_parser: &mut mpack_parser_t, node: &mut mpack_node_t) {
    unsafe {
        let node = &mut *node;
        let result = node.data[0].typval();
        match node.tok.r#type {
            mpack_token_type_t::MPACK_TOKEN_BIN | mpack_token_type_t::MPACK_TOKEN_STR => {
                let bytes = node.data[1].take_owned_bytes();
                *result = decode_string(
                    bytes.as_ptr().cast::<c_char>(),
                    node.tok.length as usize,
                    false,
                    false,
                );
            }
            mpack_token_type_t::MPACK_TOKEN_EXT => {
                let bytes = node.data[1].take_owned_bytes();
                let list = tv_list_alloc(2);
                tv_list_ref(list);
                tv_list_append_number( &mut *list, i64::from(node.tok.data.ext_type()));
                let ext_val_list = tv_list_alloc(K_LIST_LEN_MAY_KNOW);
                tv_list_append_list( &mut *list, ext_val_list);
                create_special_dict(
                    &mut *result,
                    K_MP_EXT,
                    typval_T {
                        v_type: VarType::VAR_LIST,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::List(list),
                    },
                );
                encode_list_write(
                    ext_val_list,
                    bytes.as_ptr().cast::<c_char>(),
                    node.tok.length as usize,
                );
            }
            mpack_token_type_t::MPACK_TOKEN_MAP => {
                let mut items = node.data[1].take_map_pairs();
                let len = node.tok.length as usize;
                let mut can_be_dict = items[..len].iter().all(|pair| {
                    let key = &pair[0];
                    key.v_type == VarType::VAR_STRING
                        && !key.vval.v_string().is_null()
                        && *key.vval.v_string() != 0
                });

                if can_be_dict {
                    let dict = tv_dict_alloc();
                    (*dict).dv_refcount += 1;
                    *result = typval_T {
                        v_type: VarType::VAR_DICT,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::Dict(dict),
                    };
                    for pair in &items[..len] {
                        let key = pair[0].vval.v_string();
                        let keylen = crate::memory::cstr_len(key);
                        let di = crate::eval::typval::tv_dict_item_alloc_len(key, keylen);
                        if !tv_dict_add(&mut *dict, di) {
                            let mut todo = (*dict).dv_hashtab.ht_used;
                            let mut hi = (*dict).dv_hashtab.array_ptr();
                            while todo > 0 {
                                if !hashitem_empty(&*hi) {
                                    todo -= 1;
                                    let d = crate::eval::typval::tv_dict_hi2di(hi);
                                    (*d).di_tv.v_type = VarType::VAR_SPECIAL;
                                    (*d).di_tv.vval = crate::eval::typval::TypvalVval::Number(K_SPECIAL_VAR_NULL);
                                }
                                hi = hi.add(1);
                            }
                            tv_clear(result);
                            crate::eval::typval::tv_dict_item_free(di);
                            can_be_dict = false;
                            break;
                        }
                        (*di).di_tv = pair[1];
                    }
                }

                if can_be_dict {
                    for pair in &mut items[..len] {
                        tv_clear(&raw mut pair[0]);
                    }
                } else {
                    let list = decode_create_map_special_dict(&mut *result, len as isize);
                    for pair in &items[..len] {
                        let kv_pair = tv_list_alloc(2);
                        tv_list_append_list( &mut *list, kv_pair);
                        tv_list_append_owned_tv( &mut *kv_pair, pair[0]);
                        tv_list_append_owned_tv( &mut *kv_pair, pair[1]);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Parse msgpack data into a `typval_T` using `parser` (`mpack_parse_typval`).
pub fn mpack_parse_typval(
    parser: &mut mpack_parser_t,
    data: &mut *const c_char,
    size: &mut usize,
) -> c_int {
    mpack_parse(
        parser,
        data,
        size,
        Some(typval_parse_enter),
        Some(typval_parse_exit),
    )
}

/// Unpack msgpack buffer `*data[..*size]` into `*ret` (`unpack_typval`).
pub fn unpack_typval(
    data: &mut *const c_char,
    size: &mut usize,
    ret: &mut typval_T,
) -> c_int {
    ret.v_type = VarType::VAR_UNKNOWN;
    let mut parser = mpack_parser_t::new(0);
    mpack_parser_init(&mut parser, 0);
    parser.data = mpack_data_t::Typval(ret);
    let status = mpack_parse_typval(&mut parser, data, size);
    if status != MPACK_OK {
        typval_parser_error_free(&mut parser);
        tv_clear(&raw mut *ret);
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn decode_string_copies_ascii() {
        unsafe {
            let mut tv = decode_string(c"hello", 5, false, false);
            assert_eq!(tv.v_type, VarType::VAR_STRING);
            assert_eq!(CStr::from_ptr(tv.vval.v_string()).to_str().unwrap(), "hello");
            tv_clear(&raw mut tv);
        }
    }

    #[test]
    fn json_decode_string_respects_length_and_rejects_malformed_inputs() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::globals::g_st().emsg_skip += 1;
            let mut rettv = typval_T {
                v_type: VarType::VAR_UNKNOWN,
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
                assert_eq!(rettv.v_type, VarType::VAR_UNKNOWN);
            }

            assert!(json_decode_string(c"true", 4, &mut rettv));
            assert_eq!(rettv.v_type, VarType::VAR_BOOL);
            assert_eq!(rettv.vval.v_bool(), K_BOOL_VAR_TRUE as c_int);
            tv_clear(&mut rettv);

            crate::globals::g_st().emsg_skip -= 1;
    }
}
