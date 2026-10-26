//! Encoding Vimscript `typval_T` values to strings, `:echo`, JSON, and MessagePack
//! (`src/nvim/eval/encode.c`).

use crate::api::private::converter::{K_BOOL_VAR_TRUE, K_SPECIAL_VAR_NULL};
use crate::api::private::validate::NvimString;
use crate::buffer::NOTDONE;
use crate::eval::decode::{
    K_MP_EXT, K_MP_INTEGER, K_MP_MAP, SURROGATE_FIRST_CHAR, SURROGATE_HI_END, SURROGATE_HI_START,
    SURROGATE_LO_END, SURROGATE_LO_START,
};
use crate::eval::typval::{
    TypvalVval, VAR_UNLOCKED, VarType, blob_T, dict_T, list_T, listitem_T, partial_T, tv_blob_get,
    tv_blob_len, tv_dict_find, tv_list_append_allocated_string, tv_list_first, tv_list_idx_of_item,
    tv_list_last, tv_list_len, typval_T,
};
use crate::eval::vars::{NUM_MSGPACK_TYPES, eval_msgpack_type_lists};
use crate::eval::{get_copyID, partial_name};
use crate::hashtab::{hashitem_T, hashitem_empty};
use crate::keycodes::{BS, CAR, NL, TAB};
use crate::math::{FAIL, OK};
use crate::mbyte::{utf_char2len, utf_printable, utf_ptr2char, utf_ptr2len};
use crate::memory::{memchrsub, memcnt, xfree, xmalloc, xmemdupz, xmemscan};
use crate::message::{emsg, internal_error, semsg};
use crate::msgpack_rpc::packer::{
    PackerBuffer, mpack_array, mpack_bin, mpack_bool, mpack_check_buffer, mpack_ext, mpack_float8,
    mpack_integer, mpack_map, mpack_nil, mpack_str, mpack_uint64,
};
use crate::regexp::NUMBUFLEN;
use crate::strings::{vim_snprintf, vim_snprintf_safelen};
use crate::testing::ga_concat_literal;
use std::ffi::{CStr, c_char, c_int};

pub(crate) const K_MP_NIL: usize = 0;
pub(crate) const K_MP_BOOLEAN: usize = 1;
pub(crate) const K_MP_FLOAT: usize = 3;
pub(crate) const K_MP_STRING: usize = 4;
pub(crate) const K_MP_ARRAY: usize = 5;

const FF: c_int = 0x0c;

pub const encode_bool_var_names: [*const c_char; 2] = [c"v:false".as_ptr(), c"v:true".as_ptr()];
pub const encode_special_var_names: [*const c_char; 1] = [c"v:null".as_ptr()];

static mut DID_ECHO_STRING_EMSG: bool = false;

/// Structure defining state for `encode_read_from_list()` (`ListReaderState`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ListReaderState {
    pub list: *const list_T,
    pub li: *const listitem_T,
    pub offset: usize,
    pub li_length: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MPContainerRef {
    Dict(*mut dict_T),
    List(*mut list_T),
    Pairs(*mut list_T),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MPConvPartialStage {
    Args,
    SelfDict,
    End,
}

#[derive(Clone, Copy)]
pub(crate) enum MPConvStackData {
    Dict {
        dict: *mut dict_T,
        dictp: *mut *mut dict_T,
        hi: *mut hashitem_T,
        todo: usize,
    },
    List {
        list: *mut list_T,
        li: *mut listitem_T,
    },
    Pairs {
        list: *mut list_T,
        li: *mut listitem_T,
    },
    Partial {
        stage: MPConvPartialStage,
        pt: *mut partial_T,
    },
    PartialList {
        arg: *mut typval_T,
        argv: *mut typval_T,
        todo: usize,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct MPConvStackVal {
    pub(crate) tv: *mut typval_T,
    pub(crate) saved_copy_id: c_int,
    pub(crate) data: MPConvStackData,
}

/// Msgpack callback for writing to `readfile()`-style list (`encode_list_write`).
pub unsafe fn encode_list_write(list: *mut list_T, buf: *const c_char, len: usize) {
    unsafe {
        if len == 0 {
            return;
        }
        let end = buf.add(len);
        let mut line_end = buf;
        let li = tv_list_last(list);

        if !li.is_null() {
            line_end = xmemscan(buf.cast(), NL as c_char, len)
                .cast_const()
                .cast::<c_char>();
            if line_end != buf {
                let line_length = (line_end as usize) - (buf as usize);
                let str_ptr = (*li).li_tv.vval.v_string;
                let li_len = if str_ptr.is_null() {
                    0
                } else {
                    CStr::from_ptr(str_ptr).to_bytes().len()
                };
                let new_str = crate::memory::xmallocz(li_len + line_length).cast::<c_char>();
                if !str_ptr.is_null() {
                    std::ptr::copy_nonoverlapping(str_ptr, new_str, li_len);
                    xfree(str_ptr.cast());
                }
                (*li).li_tv.vval.v_string = new_str;
                let dst = new_str.add(li_len);
                std::ptr::copy_nonoverlapping(buf.cast::<u8>(), dst.cast::<u8>(), line_length);
                memchrsub(dst.cast(), 0, NL as c_char, line_length);
            }
            line_end = line_end.add(1);
        }

        while (line_end as usize) < (end as usize) {
            let line_start = line_end;
            line_end = xmemscan(
                line_start.cast(),
                NL as c_char,
                (end as usize) - (line_start as usize),
            )
            .cast_const()
            .cast::<c_char>();
            let mut str_ptr: *mut c_char = std::ptr::null_mut();
            if line_end != line_start {
                let line_length = (line_end as usize) - (line_start as usize);
                str_ptr = xmemdupz(line_start.cast(), line_length).cast::<c_char>();
                memchrsub(str_ptr.cast(), 0, NL as c_char, line_length);
            }
            tv_list_append_allocated_string(list, str_ptr);
            line_end = line_end.add(1);
        }
        if line_end == end {
            tv_list_append_allocated_string(list, std::ptr::null_mut());
        }
    }
}

/// Initialize `ListReaderState` structure (`encode_init_lrstate`).
pub unsafe fn encode_init_lrstate(list: *const list_T) -> ListReaderState {
    let first = unsafe { tv_list_first(list) };
    let li_length = if first.is_null() {
        0
    } else {
        let s = unsafe { (*first).li_tv.vval.v_string };
        if s.is_null() {
            0
        } else {
            unsafe { CStr::from_ptr(s).to_bytes().len() }
        }
    };
    ListReaderState {
        list,
        li: first,
        offset: 0,
        li_length,
    }
}

/// Read bytes from list (`encode_read_from_list`).
pub unsafe fn encode_read_from_list(
    state: *mut ListReaderState,
    buf: *mut c_char,
    nbuf: usize,
    read_bytes: *mut usize,
) -> c_int {
    let state = unsafe { &mut *state };
    let buf_end = unsafe { buf.add(nbuf) };
    let mut p = buf;
    while (p as usize) < (buf_end as usize) {
        while state.offset < state.li_length && (p as usize) < (buf_end as usize) {
            let s = unsafe { (*state.li).li_tv.vval.v_string };
            let ch = unsafe { *s.add(state.offset) };
            state.offset += 1;
            unsafe {
                *p = if ch == (NL as c_char) { 0 } else { ch };
                p = p.add(1);
            }
        }
        if (p as usize) < (buf_end as usize) {
            state.li = unsafe { (*state.li).li_next };
            if state.li.is_null() {
                unsafe {
                    *read_bytes = (p as usize) - (buf as usize);
                }
                return OK;
            }
            unsafe {
                *p = NL as c_char;
                p = p.add(1);
            }
            if unsafe { (*state.li).li_tv.v_type } != VarType::VAR_STRING {
                unsafe {
                    *read_bytes = (p as usize) - (buf as usize);
                }
                return FAIL;
            }
            state.offset = 0;
            let s = unsafe { (*state.li).li_tv.vval.v_string };
            state.li_length = if s.is_null() {
                0
            } else {
                unsafe { CStr::from_ptr(s).to_bytes().len() }
            };
        }
    }
    unsafe {
        *read_bytes = nbuf;
    }
    if state.offset < state.li_length || unsafe { !(*state.li).li_next.is_null() } {
        NOTDONE
    } else {
        OK
    }
}

/// Convert `readfile()`-style list to a `char *` buffer with length (`encode_vim_list_to_buf`).
pub unsafe fn encode_vim_list_to_buf(
    list: *const list_T,
    ret_len: *mut usize,
    ret_buf: *mut *mut c_char,
) -> bool {
    let mut len = 0usize;
    let mut li = unsafe { tv_list_first(list) };
    while !li.is_null() {
        let tv = unsafe { &(*li).li_tv };
        if tv.v_type != VarType::VAR_STRING {
            return false;
        }
        len += 1;
        let s = unsafe { tv.vval.v_string };
        if !s.is_null() {
            len += unsafe { CStr::from_ptr(s).to_bytes().len() };
        }
        li = unsafe { (*li).li_next };
    }
    len = len.saturating_sub(1);
    unsafe {
        *ret_len = len;
    }
    if len == 0 {
        unsafe {
            *ret_buf = std::ptr::null_mut();
        }
        return true;
    }
    let mut lrstate = unsafe { encode_init_lrstate(list) };
    let buf = unsafe { xmalloc(len).cast::<c_char>() };
    let mut read_bytes = 0usize;
    if unsafe { encode_read_from_list(&mut lrstate, buf, len, &mut read_bytes) } != OK {
        std::process::abort();
    }
    assert_eq!(len, read_bytes);
    unsafe {
        *ret_buf = buf;
    }
    true
}

/// Check whether given key can be used in `json_encode()` (`encode_check_json_key`).
pub unsafe fn encode_check_json_key(tv: *const typval_T) -> bool {
    unsafe {
        let tv = &*tv;
        if tv.v_type == VarType::VAR_STRING {
            return true;
        }
        if tv.v_type != VarType::VAR_DICT {
            return false;
        }
        let spdict = tv.vval.v_dict;
        if spdict.is_null() || (*spdict).dv_hashtab.ht_used != 2 {
            return false;
        }
        let type_di = tv_dict_find(spdict, c"_TYPE".as_ptr(), 5);
        if type_di.is_null()
            || (*type_di).di_tv.v_type != VarType::VAR_LIST
            || (*type_di).di_tv.vval.v_list.cast_const() != eval_msgpack_type_lists[K_MP_STRING]
        {
            return false;
        }
        let val_di = tv_dict_find(spdict, c"_VAL".as_ptr(), 4);
        if val_di.is_null() || (*val_di).di_tv.v_type != VarType::VAR_LIST {
            return false;
        }
        let val_list = (*val_di).di_tv.vval.v_list;
        if val_list.is_null() {
            return true;
        }
        let mut li = tv_list_first(val_list);
        while !li.is_null() {
            if (*li).li_tv.v_type != VarType::VAR_STRING {
                return false;
            }
            li = (*li).li_next;
        }
        true
    }
}

#[inline]
unsafe fn buf_concat_cstr(gap: &mut Vec<u8>, s: *const c_char) {
    unsafe {
        if !s.is_null() {
            gap.extend_from_slice(CStr::from_ptr(s).to_bytes());
        }
    }
}

#[inline]
unsafe fn buf_concat_len(gap: &mut Vec<u8>, s: *const c_char, len: usize) {
    unsafe {
        if !s.is_null() && len > 0 {
            gap.extend_from_slice(std::slice::from_raw_parts(s.cast::<u8>(), len));
        }
    }
}

unsafe fn conv_error(
    msg: *const c_char,
    mpstack: &[MPConvStackVal],
    objname: *const c_char,
) -> c_int {
    unsafe {
        let mut msg_ga: Vec<u8> = Vec::with_capacity(80);
        let mut iobuf = [0 as c_char; 1025];

        for (i, v) in mpstack.iter().enumerate() {
            if i != 0 {
                ga_concat_literal(&mut msg_ga, b", ");
            }
            match v.data {
                MPConvStackData::Dict { dict, hi, .. } => {
                    let hi_ptr = if hi.is_null() {
                        (*dict).dv_hashtab.ht_array
                    } else {
                        hi.sub(1)
                    };
                    let mut key_tv = typval_T {
                        v_type: VarType::VAR_STRING,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval {
                            v_string: (*hi_ptr).hi_key,
                        },
                    };
                    let key = encode_tv2string(&mut key_tv, std::ptr::null_mut());
                    vim_snprintf!(iobuf.as_mut_ptr(), iobuf.len(), c"key %s".as_ptr(), key);
                    xfree(key.cast());
                    buf_concat_cstr(&mut msg_ga, iobuf.as_ptr());
                }
                MPConvStackData::List { list, li } | MPConvStackData::Pairs { list, li } => {
                    let is_list = matches!(v.data, MPConvStackData::List { .. });
                    let first = tv_list_first(list);
                    let idx = if li == first {
                        0
                    } else if li.is_null() {
                        tv_list_len(list) - 1
                    } else {
                        tv_list_idx_of_item(list, (*li).li_prev)
                    };
                    let prev_li: *const listitem_T = if li.is_null() {
                        tv_list_last(list)
                    } else {
                        (*li).li_prev
                    };
                    if is_list
                        || prev_li.is_null()
                        || ((*prev_li).li_tv.v_type != VarType::VAR_LIST
                            && tv_list_len((*prev_li).li_tv.vval.v_list) <= 0)
                    {
                        vim_snprintf!(iobuf.as_mut_ptr(), iobuf.len(), c"index %i".as_ptr(), idx);
                        buf_concat_cstr(&mut msg_ga, iobuf.as_ptr());
                    } else {
                        let first_item = tv_list_first((*prev_li).li_tv.vval.v_list);
                        assert!(!first_item.is_null());
                        let mut key_tv = (*first_item).li_tv;
                        let key = encode_tv2echo(&mut key_tv, std::ptr::null_mut());
                        vim_snprintf!(
                            iobuf.as_mut_ptr(),
                            iobuf.len(),
                            c"key %s at index %i from special map".as_ptr(),
                            key,
                            idx,
                        );
                        xfree(key.cast());
                        buf_concat_cstr(&mut msg_ga, iobuf.as_ptr());
                    }
                }
                MPConvStackData::Partial { stage, .. } => match stage {
                    MPConvPartialStage::Args => std::process::abort(),
                    MPConvPartialStage::SelfDict => {
                        ga_concat_literal(&mut msg_ga, b"partial");
                    }
                    MPConvPartialStage::End => {
                        ga_concat_literal(&mut msg_ga, b"partial self dictionary");
                    }
                },
                MPConvStackData::PartialList { arg, argv, .. } => {
                    let idx = (arg.offset_from(argv) as c_int) - 1;
                    vim_snprintf!(
                        iobuf.as_mut_ptr(),
                        iobuf.len(),
                        c"argument %i".as_ptr(),
                        idx,
                    );
                    buf_concat_cstr(&mut msg_ga, iobuf.as_ptr());
                }
            }
        }
        let path_ptr = if mpstack.is_empty() {
            c"itself".as_ptr()
        } else {
            msg_ga.push(0);
            msg_ga.as_ptr().cast::<c_char>()
        };
        semsg!(msg, objname, path_ptr);
        FAIL
    }
}

unsafe fn convert_to_single_quoted_string(gap: &mut Vec<u8>, buf: *const c_char, len: usize) {
    unsafe {
        if buf.is_null() {
            ga_concat_literal(gap, b"''");
        } else {
            let quotes = memcnt(buf.cast(), b'\'' as c_char, len);
            gap.reserve(2 + len + quotes);
            gap.push(b'\'');
            let bytes = std::slice::from_raw_parts(buf.cast::<u8>(), len);
            for &b in bytes {
                if b == b'\'' {
                    gap.push(b'\'');
                }
                gap.push(b);
            }
            gap.push(b'\'');
        }
    }
}

unsafe fn convert_to_json_string(gap: &mut Vec<u8>, buf: *const c_char, len: usize) -> c_int {
    const XDIGITS: &[u8; 16] = b"0123456789ABCDEF";

    unsafe {
        if buf.is_null() {
            ga_concat_literal(gap, b"\"\"");
            return OK;
        }

        let mut str_len = 0usize;
        let mut i = 0usize;
        while i < len {
            let ptr = buf.add(i);
            let ch = utf_ptr2char(ptr);
            let shift = if ch == 0 {
                1usize
            } else {
                utf_ptr2len(ptr) as usize
            };
            assert!(shift > 0);
            i += shift;
            match ch {
                BS | TAB | NL | FF | CAR | 0x22 | 0x5c => {
                    str_len += 2;
                }
                _ => {
                    if ch > 0x7F && shift == 1 {
                        let start = i - shift;
                        semsg!(
                            c"E474: String \"%.*s\" contains byte that does not start any UTF-8 character"
                                .as_ptr(),
                            (len - start) as c_int,
                            buf.add(start),
                        );
                        return FAIL;
                    } else if (SURROGATE_HI_START..=SURROGATE_HI_END).contains(&ch)
                        || (SURROGATE_LO_START..=SURROGATE_LO_END).contains(&ch)
                    {
                        let start = i - shift;
                        semsg!(
                            c"E474: UTF-8 string contains code point which belongs to a surrogate pair: %.*s"
                                .as_ptr(),
                            (len - start) as c_int,
                            buf.add(start),
                        );
                        return FAIL;
                    } else if ch >= 0x20 && utf_printable(ch) {
                        str_len += shift;
                    } else if ch >= SURROGATE_FIRST_CHAR {
                        str_len += 12;
                    } else {
                        str_len += 6;
                    }
                }
            }
        }

        gap.push(b'"');
        gap.reserve(str_len + 1);
        i = 0;
        while i < len {
            let ptr = buf.add(i);
            let ch = utf_ptr2char(ptr);
            let shift = if ch == 0 {
                1usize
            } else {
                utf_char2len(ch) as usize
            };
            assert!(shift > 0);
            match ch {
                BS => ga_concat_literal(gap, b"\\b"),
                TAB => ga_concat_literal(gap, b"\\t"),
                NL => ga_concat_literal(gap, b"\\n"),
                FF => ga_concat_literal(gap, b"\\f"),
                CAR => ga_concat_literal(gap, b"\\r"),
                0x22 => ga_concat_literal(gap, b"\\\""),
                0x5c => ga_concat_literal(gap, b"\\\\"),
                _ => {
                    if ch >= 0x20 && utf_printable(ch) {
                        buf_concat_len(gap, ptr, shift);
                    } else if ch < SURROGATE_FIRST_CHAR {
                        let esc = [
                            b'\\',
                            b'u',
                            XDIGITS[((ch >> 12) & 0xF) as usize],
                            XDIGITS[((ch >> 8) & 0xF) as usize],
                            XDIGITS[((ch >> 4) & 0xF) as usize],
                            XDIGITS[(ch & 0xF) as usize],
                        ];
                        ga_concat_literal(gap, &esc);
                    } else {
                        let tmp = ch - SURROGATE_FIRST_CHAR;
                        let hi = SURROGATE_HI_START + ((tmp >> 10) & 0x3FF);
                        let lo = SURROGATE_LO_END + (tmp & 0x3FF);
                        let esc = [
                            b'\\',
                            b'u',
                            XDIGITS[((hi >> 12) & 0xF) as usize],
                            XDIGITS[((hi >> 8) & 0xF) as usize],
                            XDIGITS[((hi >> 4) & 0xF) as usize],
                            XDIGITS[(hi & 0xF) as usize],
                            b'\\',
                            b'u',
                            XDIGITS[((lo >> 12) & 0xF) as usize],
                            XDIGITS[((lo >> 8) & 0xF) as usize],
                            XDIGITS[((lo >> 4) & 0xF) as usize],
                            XDIGITS[(lo & 0xF) as usize],
                        ];
                        ga_concat_literal(gap, &esc);
                    }
                }
            }
            i += shift;
        }
        gap.push(b'"');
        OK
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextEncodeMode {
    String,
    Echo,
    Json,
}

enum EncodeBackend<'a> {
    Text {
        gap: &'a mut Vec<u8>,
        mode: TextEncodeMode,
    },
    Msgpack {
        packer: *mut PackerBuffer,
    },
}

impl EncodeBackend<'_> {
    #[inline]
    fn allow_specials(&self) -> bool {
        matches!(
            self,
            Self::Text {
                mode: TextEncodeMode::Json,
                ..
            } | Self::Msgpack { .. }
        )
    }

    #[inline]
    fn convert_one_value_err_name(&self) -> *const c_char {
        match self {
            Self::Text {
                mode: TextEncodeMode::String,
                ..
            } => c"_typval_encode_string_convert_one_value()".as_ptr(),
            Self::Text {
                mode: TextEncodeMode::Echo,
                ..
            } => c"_typval_encode_echo_convert_one_value()".as_ptr(),
            Self::Text {
                mode: TextEncodeMode::Json,
                ..
            } => c"_typval_encode_json_convert_one_value()".as_ptr(),
            Self::Msgpack { .. } => c"_typval_encode_msgpack_convert_one_value()".as_ptr(),
        }
    }

    #[inline]
    unsafe fn check_before(&mut self) {
        if let Self::Msgpack { packer } = *self {
            unsafe {
                mpack_check_buffer(packer);
            }
        }
    }

    #[inline]
    unsafe fn conv_nil(&mut self) {
        unsafe {
            match self {
                Self::Text {
                    gap,
                    mode: TextEncodeMode::String | TextEncodeMode::Echo,
                } => ga_concat_literal(gap, b"v:null"),
                Self::Text {
                    gap,
                    mode: TextEncodeMode::Json,
                } => ga_concat_literal(gap, b"null"),
                Self::Msgpack { packer } => mpack_nil(&mut (**packer).ptr),
            }
        }
    }

    #[inline]
    unsafe fn conv_bool(&mut self, val: bool) {
        unsafe {
            match self {
                Self::Text {
                    gap,
                    mode: TextEncodeMode::String | TextEncodeMode::Echo,
                } => {
                    if val {
                        ga_concat_literal(gap, b"v:true");
                    } else {
                        ga_concat_literal(gap, b"v:false");
                    }
                }
                Self::Text {
                    gap,
                    mode: TextEncodeMode::Json,
                } => {
                    if val {
                        ga_concat_literal(gap, b"true");
                    } else {
                        ga_concat_literal(gap, b"false");
                    }
                }
                Self::Msgpack { packer } => mpack_bool(&mut (**packer).ptr, val),
            }
        }
    }

    #[inline]
    unsafe fn conv_number(&mut self, num: i64) {
        unsafe {
            match self {
                Self::Text { gap, .. } => {
                    let s = num.to_string();
                    ga_concat_literal(gap, s.as_bytes());
                }
                Self::Msgpack { packer } => mpack_integer(&mut (**packer).ptr, num),
            }
        }
    }

    #[inline]
    unsafe fn conv_unsigned_number(&mut self, num: u64) {
        unsafe {
            match self {
                Self::Text {
                    gap,
                    mode: TextEncodeMode::Json,
                } => {
                    let s = num.to_string();
                    ga_concat_literal(gap, s.as_bytes());
                }
                Self::Text { .. } => {}
                Self::Msgpack { packer } => mpack_uint64(&mut (**packer).ptr, num),
            }
        }
    }

    #[inline]
    unsafe fn conv_float(&mut self, flt: f64) -> c_int {
        unsafe {
            match self {
                Self::Text {
                    gap,
                    mode: TextEncodeMode::String | TextEncodeMode::Echo,
                } => {
                    if flt.is_nan() {
                        ga_concat_literal(gap, b"str2float('nan')");
                    } else if flt.is_infinite() {
                        if flt.is_sign_negative() {
                            gap.push(b'-');
                        }
                        ga_concat_literal(gap, b"str2float('inf')");
                    } else {
                        let mut numbuf = [0 as c_char; NUMBUFLEN];
                        let len = vim_snprintf_safelen!(
                            numbuf.as_mut_ptr(),
                            numbuf.len(),
                            c"%g".as_ptr(),
                            flt,
                        );
                        buf_concat_len(gap, numbuf.as_ptr(), len);
                    }
                    OK
                }
                Self::Text {
                    gap,
                    mode: TextEncodeMode::Json,
                } => {
                    if flt.is_nan() {
                        emsg(c"E474: Unable to represent NaN value in JSON".as_ptr());
                        return FAIL;
                    } else if flt.is_infinite() {
                        emsg(c"E474: Unable to represent infinity in JSON".as_ptr());
                        return FAIL;
                    } else {
                        let mut numbuf = [0 as c_char; NUMBUFLEN];
                        let len = vim_snprintf_safelen!(
                            numbuf.as_mut_ptr(),
                            numbuf.len(),
                            c"%g".as_ptr(),
                            flt,
                        );
                        buf_concat_len(gap, numbuf.as_ptr(), len);
                    }
                    OK
                }
                Self::Msgpack { packer } => {
                    mpack_float8(&mut (**packer).ptr, flt);
                    OK
                }
            }
        }
    }

    #[inline]
    unsafe fn conv_string(&mut self, buf: *const c_char, len: usize) -> c_int {
        unsafe {
            match self {
                Self::Text {
                    gap,
                    mode: TextEncodeMode::String | TextEncodeMode::Echo,
                } => {
                    convert_to_single_quoted_string(gap, buf, len);
                    OK
                }
                Self::Text {
                    gap,
                    mode: TextEncodeMode::Json,
                } => convert_to_json_string(gap, buf, len),
                Self::Msgpack { packer } => {
                    mpack_bin(
                        NvimString {
                            data: buf.cast_mut(),
                            size: len,
                        },
                        *packer,
                    );
                    OK
                }
            }
        }
    }

    #[inline]
    unsafe fn conv_str_string(&mut self, buf: *const c_char, len: usize) -> c_int {
        unsafe {
            match self {
                Self::Text { .. } => self.conv_string(buf, len),
                Self::Msgpack { packer } => {
                    mpack_str(
                        NvimString {
                            data: buf.cast_mut(),
                            size: len,
                        },
                        *packer,
                    );
                    OK
                }
            }
        }
    }

    #[inline]
    unsafe fn conv_ext_string(&mut self, buf: *mut c_char, len: usize, ext_type: i8) -> c_int {
        unsafe {
            match self {
                Self::Text {
                    mode: TextEncodeMode::Json,
                    ..
                } => {
                    emsg(c"E474: Unable to convert EXT string to JSON".as_ptr());
                    FAIL
                }
                Self::Text { .. } => OK,
                Self::Msgpack { packer } => {
                    mpack_ext(buf, len, ext_type, *packer);
                    OK
                }
            }
        }
    }

    #[inline]
    unsafe fn conv_blob(&mut self, blob: *const blob_T, len: c_int) {
        unsafe {
            match self {
                Self::Text {
                    gap,
                    mode: TextEncodeMode::String | TextEncodeMode::Echo,
                } => {
                    if len == 0 {
                        ga_concat_literal(gap, b"0z");
                    } else {
                        gap.reserve((2 + 2 * len + (len - 1) / 4) as usize);
                        ga_concat_literal(gap, b"0z");
                        let mut numbuf = [0 as c_char; NUMBUFLEN];
                        for i in 0..len {
                            if i > 0 && (i & 3) == 0 {
                                gap.push(b'.');
                            }
                            let nlen = vim_snprintf_safelen!(
                                numbuf.as_mut_ptr(),
                                numbuf.len(),
                                c"%02X".as_ptr(),
                                tv_blob_get(blob, i) as c_int,
                            );
                            buf_concat_len(gap, numbuf.as_ptr(), nlen);
                        }
                    }
                }
                Self::Text {
                    gap,
                    mode: TextEncodeMode::Json,
                } => {
                    if len == 0 {
                        ga_concat_literal(gap, b"[]");
                    } else {
                        gap.push(b'[');
                        let mut numbuf = [0 as c_char; NUMBUFLEN];
                        for i in 0..len {
                            if i > 0 {
                                ga_concat_literal(gap, b", ");
                            }
                            let nlen = vim_snprintf_safelen!(
                                numbuf.as_mut_ptr(),
                                numbuf.len(),
                                c"%d".as_ptr(),
                                tv_blob_get(blob, i) as c_int,
                            );
                            buf_concat_len(gap, numbuf.as_ptr(), nlen);
                        }
                        gap.push(b']');
                    }
                }
                Self::Msgpack { packer } => {
                    let data = if blob.is_null() || (*blob).bv_ga.is_empty() {
                        std::ptr::null_mut()
                    } else {
                        (*blob).bv_ga.as_ptr().cast_mut().cast::<c_char>()
                    };
                    mpack_bin(
                        NvimString {
                            data,
                            size: len as usize,
                        },
                        *packer,
                    );
                }
            }
        }
    }

    #[inline]
    unsafe fn conv_func_start(
        &mut self,
        fun: *const c_char,
        prefix: &[u8],
        mpstack: &[MPConvStackVal],
        objname: *const c_char,
    ) -> c_int {
        unsafe {
            match self {
                Self::Text {
                    gap,
                    mode: TextEncodeMode::String | TextEncodeMode::Echo,
                } => {
                    if fun.is_null() {
                        internal_error(c"string(): NULL function name".as_ptr());
                        ga_concat_literal(gap, b"function(NULL");
                    } else {
                        ga_concat_literal(gap, b"function(");
                        let name_off = gap.len();
                        ga_concat_literal(gap, prefix);
                        let fun_len = CStr::from_ptr(fun).to_bytes().len();
                        convert_to_single_quoted_string(gap, fun, fun_len);
                        let data = gap.as_mut_ptr();
                        *data.add(name_off) = b'\'';
                        if !prefix.is_empty() {
                            std::ptr::copy_nonoverlapping(
                                prefix.as_ptr(),
                                data.add(name_off + 1),
                                prefix.len(),
                            );
                        }
                    }
                    OK
                }
                Self::Text {
                    mode: TextEncodeMode::Json,
                    ..
                } => conv_error(
                    c"E474: Error while dumping %s, %s: attempt to dump function reference"
                        .as_ptr(),
                    mpstack,
                    objname,
                ),
                Self::Msgpack { .. } => conv_error(
                    c"E5004: Error while dumping %s, %s: attempt to dump function reference"
                        .as_ptr(),
                    mpstack,
                    objname,
                ),
            }
        }
    }

    #[inline]
    unsafe fn conv_func_before_args(&mut self, len: c_int) {
        if let Self::Text { gap, .. } = self
            && len != 0
        {
            ga_concat_literal(gap, b", ");
        }
    }

    #[inline]
    unsafe fn conv_func_before_self(&mut self, len: isize) {
        if let Self::Text { gap, .. } = self
            && len != -1
        {
            ga_concat_literal(gap, b", ");
        }
    }

    #[inline]
    unsafe fn conv_func_end(&mut self) {
        if let Self::Text { gap, .. } = self {
            gap.push(b')');
        }
    }

    #[inline]
    unsafe fn conv_empty_list(&mut self) {
        unsafe {
            match self {
                Self::Text { gap, .. } => ga_concat_literal(gap, b"[]"),
                Self::Msgpack { packer } => mpack_array(&mut (**packer).ptr, 0),
            }
        }
    }

    #[inline]
    unsafe fn conv_list_start(&mut self, len: c_int) {
        unsafe {
            match self {
                Self::Text { gap, .. } => gap.push(b'['),
                Self::Msgpack { packer } => mpack_array(&mut (**packer).ptr, len as u32),
            }
        }
    }

    #[inline]
    unsafe fn conv_list_between_items(&mut self) {
        if let Self::Text { gap, .. } = self {
            ga_concat_literal(gap, b", ");
        }
    }

    #[inline]
    unsafe fn conv_list_end(&mut self) {
        if let Self::Text { gap, .. } = self {
            gap.push(b']');
        }
    }

    #[inline]
    unsafe fn conv_empty_dict(&mut self) {
        unsafe {
            match self {
                Self::Text { gap, .. } => ga_concat_literal(gap, b"{}"),
                Self::Msgpack { packer } => mpack_map(&mut (**packer).ptr, 0),
            }
        }
    }

    #[inline]
    unsafe fn conv_dict_start(&mut self, len: usize) {
        unsafe {
            match self {
                Self::Text { gap, .. } => gap.push(b'{'),
                Self::Msgpack { packer } => mpack_map(&mut (**packer).ptr, len as u32),
            }
        }
    }

    #[inline]
    unsafe fn conv_dict_after_key(&mut self) {
        if let Self::Text { gap, .. } = self {
            ga_concat_literal(gap, b": ");
        }
    }

    #[inline]
    unsafe fn conv_dict_between_items(&mut self) {
        if let Self::Text { gap, .. } = self {
            ga_concat_literal(gap, b", ");
        }
    }

    #[inline]
    unsafe fn conv_dict_end(&mut self) {
        if let Self::Text { gap, .. } = self {
            gap.push(b'}');
        }
    }

    #[inline]
    unsafe fn special_dict_key_check(&mut self, key: &typval_T) -> bool {
        if matches!(
            *self,
            Self::Text {
                mode: TextEncodeMode::Json,
                ..
            }
        ) && unsafe { !encode_check_json_key(key) }
        {
            unsafe {
                emsg(c"E474: Invalid key in special dictionary".as_ptr());
            }
            return false;
        }
        true
    }

    unsafe fn check_self_reference(
        &mut self,
        container: MPContainerRef,
        val_copy_id: *mut c_int,
        mpstack: &[MPConvStackVal],
        copy_id: c_int,
        objname: *const c_char,
    ) -> c_int {
        unsafe {
            if *val_copy_id != copy_id {
                *val_copy_id = copy_id;
                return NOTDONE;
            }
            let find_backref = || -> usize {
                for (idx, mpval) in mpstack.iter().enumerate() {
                    match (container, mpval.data) {
                        (MPContainerRef::Dict(d1), MPConvStackData::Dict { dict: d2, .. })
                            if d1 == d2 =>
                        {
                            return idx;
                        }
                        (MPContainerRef::List(l1), MPConvStackData::List { list: l2, .. })
                            if l1 == l2 =>
                        {
                            return idx;
                        }
                        _ => {}
                    }
                }
                mpstack.len()
            };

            match self {
                Self::Text {
                    gap,
                    mode: TextEncodeMode::String,
                } => {
                    if !DID_ECHO_STRING_EMSG {
                        DID_ECHO_STRING_EMSG = true;
                        emsg(
                            c"E724: unable to correctly dump variable with self-referencing container"
                                .as_ptr(),
                        );
                    }
                    let backref = find_backref();
                    let s = format!("{{E724@{backref}}}");
                    ga_concat_literal(gap, s.as_bytes());
                    OK
                }
                Self::Text {
                    gap,
                    mode: TextEncodeMode::Echo,
                } => {
                    let backref = find_backref();
                    let s = if matches!(container, MPContainerRef::Dict(_)) {
                        format!("{{...@{backref}}}")
                    } else {
                        format!("[...@{backref}]")
                    };
                    ga_concat_literal(gap, s.as_bytes());
                    OK
                }
                Self::Text {
                    mode: TextEncodeMode::Json,
                    ..
                } => {
                    if !DID_ECHO_STRING_EMSG {
                        DID_ECHO_STRING_EMSG = true;
                        emsg(
                            c"E724: unable to correctly dump variable with self-referencing container"
                                .as_ptr(),
                        );
                    }
                    OK
                }
                Self::Msgpack { .. } => conv_error(
                    c"E5005: Unable to dump %s: container references itself in %s".as_ptr(),
                    mpstack,
                    objname,
                ),
            }
        }
    }
}

unsafe fn convert_one_value(
    backend: &mut EncodeBackend,
    mpstack: &mut Vec<MPConvStackVal>,
    tv: *mut typval_T,
    copy_id: c_int,
    objname: *const c_char,
) -> c_int {
    unsafe {
        backend.check_before();
        let tv_ref = &mut *tv;
        match tv_ref.v_type {
            VarType::VAR_STRING => {
                let s = tv_ref.vval.v_string;
                let len = if s.is_null() {
                    0
                } else {
                    CStr::from_ptr(s).to_bytes().len()
                };
                if backend.conv_string(s, len) == FAIL {
                    return FAIL;
                }
            }
            VarType::VAR_NUMBER => {
                backend.conv_number(tv_ref.vval.v_number);
            }
            VarType::VAR_FLOAT => {
                if backend.conv_float(tv_ref.vval.v_float) == FAIL {
                    return FAIL;
                }
            }
            VarType::VAR_BLOB => {
                let blob = tv_ref.vval.v_blob;
                backend.conv_blob(blob, tv_blob_len(blob));
            }
            VarType::VAR_FUNC => {
                if backend.conv_func_start(tv_ref.vval.v_string, b"", mpstack, objname) == FAIL {
                    return FAIL;
                }
                backend.conv_func_before_args(0);
                backend.conv_func_before_self(-1);
                backend.conv_func_end();
            }
            VarType::VAR_PARTIAL => {
                let pt = tv_ref.vval.v_partial;
                let fun = if pt.is_null() {
                    std::ptr::null_mut()
                } else {
                    partial_name(pt)
                };
                let prefix: &[u8] = if !fun.is_null()
                    && !pt.is_null()
                    && (*pt).pt_name.is_null()
                    && ((*fun as u8).is_ascii_uppercase())
                {
                    b"g:"
                } else {
                    b""
                };
                if backend.conv_func_start(fun, prefix, mpstack, objname) == FAIL {
                    return FAIL;
                }
                mpstack.push(MPConvStackVal {
                    tv,
                    saved_copy_id: copy_id - 1,
                    data: MPConvStackData::Partial {
                        stage: MPConvPartialStage::Args,
                        pt,
                    },
                });
            }
            VarType::VAR_LIST => {
                let list = tv_ref.vval.v_list;
                if list.is_null() || tv_list_len(list) == 0 {
                    backend.conv_empty_list();
                } else {
                    let saved_copy_id = (*list).lv_copyID;
                    let csr = backend.check_self_reference(
                        MPContainerRef::List(list),
                        &mut (*list).lv_copyID,
                        mpstack,
                        copy_id,
                        objname,
                    );
                    if csr != NOTDONE {
                        return csr;
                    }
                    backend.conv_list_start(tv_list_len(list));
                    mpstack.push(MPConvStackVal {
                        tv,
                        saved_copy_id,
                        data: MPConvStackData::List {
                            list,
                            li: tv_list_first(list),
                        },
                    });
                }
            }
            VarType::VAR_BOOL => {
                backend.conv_bool(tv_ref.vval.v_bool == K_BOOL_VAR_TRUE as c_int);
            }
            VarType::VAR_SPECIAL => {
                if tv_ref.vval.v_special == K_SPECIAL_VAR_NULL as c_int {
                    backend.conv_nil();
                }
            }
            VarType::VAR_DICT => {
                let dict = tv_ref.vval.v_dict;
                if dict.is_null() || (*dict).dv_hashtab.ht_used == 0 {
                    backend.conv_empty_dict();
                    return OK;
                }
                let mut handled_special = false;
                if backend.allow_specials() && (*dict).dv_hashtab.ht_used == 2 {
                    let type_di = tv_dict_find(dict, c"_TYPE".as_ptr(), 5);
                    if !type_di.is_null() && (*type_di).di_tv.v_type == VarType::VAR_LIST {
                        let val_di = tv_dict_find(dict, c"_VAL".as_ptr(), 4);
                        if !val_di.is_null() {
                            let mut mp_idx = NUM_MSGPACK_TYPES;
                            for (i, tl) in { eval_msgpack_type_lists }.into_iter().enumerate() {
                                if (*type_di).di_tv.vval.v_list.cast_const() == tl {
                                    mp_idx = i;
                                    break;
                                }
                            }
                            backend.check_before();
                            if mp_idx < NUM_MSGPACK_TYPES {
                                match mp_idx {
                                    K_MP_NIL => {
                                        backend.conv_nil();
                                        handled_special = true;
                                    }
                                    K_MP_BOOLEAN => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_NUMBER {
                                            backend.conv_bool((*val_di).di_tv.vval.v_number != 0);
                                            handled_special = true;
                                        }
                                    }
                                    K_MP_INTEGER => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_LIST {
                                            let val_list = (*val_di).di_tv.vval.v_list;
                                            if tv_list_len(val_list) == 4 {
                                                let sign_li = tv_list_first(val_list);
                                                let highest_li = (*sign_li).li_next;
                                                let high_li = (*highest_li).li_next;
                                                let low_li = tv_list_last(val_list);
                                                if (*sign_li).li_tv.v_type == VarType::VAR_NUMBER
                                                    && (*sign_li).li_tv.vval.v_number != 0
                                                    && (*highest_li).li_tv.v_type
                                                        == VarType::VAR_NUMBER
                                                    && (*highest_li).li_tv.vval.v_number >= 0
                                                    && (*high_li).li_tv.v_type
                                                        == VarType::VAR_NUMBER
                                                    && (*high_li).li_tv.vval.v_number >= 0
                                                    && (*low_li).li_tv.v_type == VarType::VAR_NUMBER
                                                    && (*low_li).li_tv.vval.v_number >= 0
                                                {
                                                    let sign = (*sign_li).li_tv.vval.v_number;
                                                    let highest =
                                                        (*highest_li).li_tv.vval.v_number as u64;
                                                    let high =
                                                        (*high_li).li_tv.vval.v_number as u64;
                                                    let low = (*low_li).li_tv.vval.v_number as u64;
                                                    let number =
                                                        (highest << 62) | (high << 31) | low;
                                                    if sign > 0 {
                                                        backend.conv_unsigned_number(number);
                                                    } else {
                                                        backend.conv_number(
                                                            number.wrapping_neg() as i64
                                                        );
                                                    }
                                                    handled_special = true;
                                                }
                                            }
                                        }
                                    }
                                    K_MP_FLOAT => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_FLOAT {
                                            if backend.conv_float((*val_di).di_tv.vval.v_float)
                                                == FAIL
                                            {
                                                return FAIL;
                                            }
                                            handled_special = true;
                                        }
                                    }
                                    K_MP_STRING => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_LIST {
                                            let mut len = 0usize;
                                            let mut buf: *mut c_char = std::ptr::null_mut();
                                            if encode_vim_list_to_buf(
                                                (*val_di).di_tv.vval.v_list,
                                                &mut len,
                                                &mut buf,
                                            ) {
                                                let res = backend.conv_str_string(buf, len);
                                                xfree(buf.cast());
                                                if res == FAIL {
                                                    return FAIL;
                                                }
                                                handled_special = true;
                                            }
                                        }
                                    }
                                    K_MP_ARRAY => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_LIST {
                                            let val_list = (*val_di).di_tv.vval.v_list;
                                            let saved_copy_id = (*val_list).lv_copyID;
                                            let csr = backend.check_self_reference(
                                                MPContainerRef::List(val_list),
                                                &mut (*val_list).lv_copyID,
                                                mpstack,
                                                copy_id,
                                                objname,
                                            );
                                            if csr != NOTDONE {
                                                return csr;
                                            }
                                            backend.conv_list_start(tv_list_len(val_list));
                                            mpstack.push(MPConvStackVal {
                                                tv,
                                                saved_copy_id,
                                                data: MPConvStackData::List {
                                                    list: val_list,
                                                    li: tv_list_first(val_list),
                                                },
                                            });
                                            handled_special = true;
                                        }
                                    }
                                    K_MP_MAP => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_LIST {
                                            let val_list = (*val_di).di_tv.vval.v_list;
                                            if val_list.is_null() || tv_list_len(val_list) == 0 {
                                                backend.conv_empty_dict();
                                                handled_special = true;
                                            } else {
                                                let mut all_pairs = true;
                                                let mut li = tv_list_first(val_list);
                                                while !li.is_null() {
                                                    if (*li).li_tv.v_type != VarType::VAR_LIST
                                                        || tv_list_len((*li).li_tv.vval.v_list) != 2
                                                    {
                                                        all_pairs = false;
                                                        break;
                                                    }
                                                    li = (*li).li_next;
                                                }
                                                if all_pairs {
                                                    let saved_copy_id = (*val_list).lv_copyID;
                                                    let csr = backend.check_self_reference(
                                                        MPContainerRef::Pairs(val_list),
                                                        &mut (*val_list).lv_copyID,
                                                        mpstack,
                                                        copy_id,
                                                        objname,
                                                    );
                                                    if csr != NOTDONE {
                                                        return csr;
                                                    }
                                                    backend.conv_dict_start(
                                                        tv_list_len(val_list) as usize
                                                    );
                                                    mpstack.push(MPConvStackVal {
                                                        tv,
                                                        saved_copy_id,
                                                        data: MPConvStackData::Pairs {
                                                            list: val_list,
                                                            li: tv_list_first(val_list),
                                                        },
                                                    });
                                                    handled_special = true;
                                                }
                                            }
                                        }
                                    }
                                    K_MP_EXT if (*val_di).di_tv.v_type == VarType::VAR_LIST => {
                                        let val_list = (*val_di).di_tv.vval.v_list;
                                        if tv_list_len(val_list) == 2 {
                                            let first = tv_list_first(val_list);
                                            let last = tv_list_last(val_list);
                                            if (*first).li_tv.v_type == VarType::VAR_NUMBER
                                                && (i8::MIN as i64..=i8::MAX as i64)
                                                    .contains(&(*first).li_tv.vval.v_number)
                                                && (*last).li_tv.v_type == VarType::VAR_LIST
                                            {
                                                let ext_type = (*first).li_tv.vval.v_number as i8;
                                                let mut len = 0usize;
                                                let mut buf: *mut c_char = std::ptr::null_mut();
                                                if encode_vim_list_to_buf(
                                                    (*last).li_tv.vval.v_list,
                                                    &mut len,
                                                    &mut buf,
                                                ) {
                                                    let res =
                                                        backend.conv_ext_string(buf, len, ext_type);
                                                    xfree(buf.cast());
                                                    if res == FAIL {
                                                        return FAIL;
                                                    }
                                                    handled_special = true;
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                if !handled_special {
                    let saved_copy_id = (*dict).dv_copyID;
                    let csr = backend.check_self_reference(
                        MPContainerRef::Dict(dict),
                        &mut (*dict).dv_copyID,
                        mpstack,
                        copy_id,
                        objname,
                    );
                    if csr != NOTDONE {
                        return csr;
                    }
                    backend.conv_dict_start((*dict).dv_hashtab.ht_used);
                    mpstack.push(MPConvStackVal {
                        tv,
                        saved_copy_id,
                        data: MPConvStackData::Dict {
                            dict,
                            dictp: std::ptr::addr_of_mut!(tv_ref.vval.v_dict),
                            hi: (*dict).dv_hashtab.ht_array,
                            todo: (*dict).dv_hashtab.ht_used,
                        },
                    });
                }
            }
            VarType::VAR_UNKNOWN => {
                internal_error(backend.convert_one_value_err_name());
                return FAIL;
            }
        }
        OK
    }
}

unsafe fn encode_vim_with_backend(
    backend: &mut EncodeBackend,
    top_tv: *mut typval_T,
    objname: *const c_char,
) -> c_int {
    unsafe {
        let copy_id = get_copyID();
        let mut mpstack: Vec<MPConvStackVal> = Vec::with_capacity(8);

        if convert_one_value(backend, &mut mpstack, top_tv, copy_id, objname) == FAIL {
            return FAIL;
        }

        while !mpstack.is_empty() {
            let cur_idx = mpstack.len() - 1;
            let saved_copy_id = mpstack[cur_idx].saved_copy_id;
            let next_tv: *mut typval_T = match mpstack[cur_idx].data {
                MPConvStackData::Dict {
                    dict,
                    mut hi,
                    ref mut todo,
                    ..
                } => {
                    if *todo == 0 {
                        mpstack.pop();
                        (*dict).dv_copyID = saved_copy_id;
                        backend.conv_dict_end();
                        continue;
                    } else if *todo != (*dict).dv_hashtab.ht_used {
                        backend.conv_dict_between_items();
                    }
                    while hashitem_empty(&*hi) {
                        hi = hi.add(1);
                    }
                    let di = crate::eval::typval::tv_dict_hi2di(hi);
                    *todo -= 1;
                    if let MPConvStackData::Dict {
                        hi: ref mut hi_ref, ..
                    } = mpstack[cur_idx].data
                    {
                        *hi_ref = hi.add(1);
                    }
                    let key_ptr = (*di).di_key.as_ptr();
                    let key_len = CStr::from_ptr(key_ptr).to_bytes().len();
                    if backend.conv_str_string(key_ptr, key_len) == FAIL {
                        return FAIL;
                    }
                    backend.conv_dict_after_key();
                    &mut (*di).di_tv
                }
                MPConvStackData::List { list, ref mut li } => {
                    let cur_li = *li;
                    if cur_li.is_null() {
                        mpstack.pop();
                        (*list).lv_copyID = saved_copy_id;
                        backend.conv_list_end();
                        continue;
                    } else if cur_li != tv_list_first(list) {
                        backend.conv_list_between_items();
                    }
                    *li = (*cur_li).li_next;
                    &mut (*cur_li).li_tv
                }
                MPConvStackData::Pairs { list, li } => {
                    if li.is_null() {
                        mpstack.pop();
                        (*list).lv_copyID = saved_copy_id;
                        backend.conv_dict_end();
                        continue;
                    } else if li != tv_list_first(list) {
                        backend.conv_dict_between_items();
                    }
                    let kv_pair = (*li).li_tv.vval.v_list;
                    let first_item = tv_list_first(kv_pair);
                    if !backend.special_dict_key_check(&(*first_item).li_tv) {
                        return FAIL;
                    }
                    if convert_one_value(
                        backend,
                        &mut mpstack,
                        &mut (*first_item).li_tv,
                        copy_id,
                        objname,
                    ) == FAIL
                    {
                        return FAIL;
                    }
                    backend.conv_dict_after_key();
                    let last_item = tv_list_last(kv_pair);
                    let next_li = (*li).li_next;
                    if let MPConvStackData::Pairs {
                        li: ref mut li_ref, ..
                    } = mpstack[cur_idx].data
                    {
                        *li_ref = next_li;
                    }
                    &mut (*last_item).li_tv
                }
                MPConvStackData::Partial { stage, pt } => {
                    match stage {
                        MPConvPartialStage::Args => {
                            let argc = if pt.is_null() {
                                0
                            } else {
                                (*pt).pt_argv.len() as c_int
                            };
                            backend.conv_func_before_args(argc);
                            if let MPConvStackData::Partial {
                                stage: ref mut stage_ref,
                                ..
                            } = mpstack[cur_idx].data
                            {
                                *stage_ref = MPConvPartialStage::SelfDict;
                            }
                            if argc > 0 {
                                backend.conv_list_start(argc);
                                mpstack.push(MPConvStackVal {
                                    tv: std::ptr::null_mut(),
                                    saved_copy_id: copy_id - 1,
                                    data: MPConvStackData::PartialList {
                                        arg: (*pt).pt_argv.as_mut_ptr(),
                                        argv: (*pt).pt_argv.as_mut_ptr(),
                                        todo: argc as usize,
                                    },
                                });
                            }
                        }
                        MPConvPartialStage::SelfDict => {
                            if let MPConvStackData::Partial {
                                stage: ref mut stage_ref,
                                ..
                            } = mpstack[cur_idx].data
                            {
                                *stage_ref = MPConvPartialStage::End;
                            }
                            let dict = if pt.is_null() {
                                std::ptr::null_mut()
                            } else {
                                (*pt).pt_dict
                            };
                            if !dict.is_null() {
                                backend.conv_func_before_self((*dict).dv_hashtab.ht_used as isize);
                                if (*dict).dv_hashtab.ht_used == 0 {
                                    backend.conv_empty_dict();
                                    continue;
                                }
                                let saved_copy_id = (*dict).dv_copyID;
                                let csr = backend.check_self_reference(
                                    MPContainerRef::Dict(dict),
                                    &mut (*dict).dv_copyID,
                                    &mpstack,
                                    copy_id,
                                    objname,
                                );
                                if csr != NOTDONE {
                                    if csr == FAIL {
                                        return FAIL;
                                    } else {
                                        continue;
                                    }
                                }
                                backend.conv_dict_start((*dict).dv_hashtab.ht_used);
                                mpstack.push(MPConvStackVal {
                                    tv: std::ptr::null_mut(),
                                    saved_copy_id,
                                    data: MPConvStackData::Dict {
                                        dict,
                                        dictp: std::ptr::addr_of_mut!((*pt).pt_dict),
                                        hi: (*dict).dv_hashtab.ht_array,
                                        todo: (*dict).dv_hashtab.ht_used,
                                    },
                                });
                            } else {
                                backend.conv_func_before_self(-1);
                            }
                        }
                        MPConvPartialStage::End => {
                            backend.conv_func_end();
                            mpstack.pop();
                        }
                    }
                    continue;
                }
                MPConvStackData::PartialList {
                    ref mut arg,
                    argv,
                    ref mut todo,
                } => {
                    let cur_arg = *arg;
                    if *todo == 0 {
                        mpstack.pop();
                        backend.conv_list_end();
                        continue;
                    } else if argv != cur_arg {
                        backend.conv_list_between_items();
                    }
                    *arg = cur_arg.add(1);
                    *todo -= 1;
                    cur_arg
                }
            };

            if convert_one_value(backend, &mut mpstack, next_tv, copy_id, objname) == FAIL {
                return FAIL;
            }
        }

        OK
    }
}

/// Convert Vimscript value to `:echo` output (`encode_vim_to_echo`).
pub unsafe fn encode_vim_to_echo(
    gap: &mut Vec<u8>,
    tv: *mut typval_T,
    objname: *const c_char,
) -> c_int {
    unsafe {
        let mut backend = EncodeBackend::Text {
            gap,
            mode: TextEncodeMode::Echo,
        };
        encode_vim_with_backend(&mut backend, tv, objname)
    }
}

/// Convert Vimscript value to msgpack string (`encode_vim_to_msgpack`).
pub unsafe fn encode_vim_to_msgpack(
    packer: *mut PackerBuffer,
    tv: *mut typval_T,
    objname: *const c_char,
) -> c_int {
    unsafe {
        let mut backend = EncodeBackend::Msgpack { packer };
        encode_vim_with_backend(&mut backend, tv, objname)
    }
}

/// Return a string with the string representation of a variable (`encode_tv2string`).
pub unsafe fn encode_tv2string(tv: *mut typval_T, len: *mut usize) -> *mut c_char {
    unsafe {
        let mut ga: Vec<u8> = Vec::with_capacity(80);
        let mut backend = EncodeBackend::Text {
            gap: &mut ga,
            mode: TextEncodeMode::String,
        };
        let evs_ret =
            encode_vim_with_backend(&mut backend, tv, c"encode_tv2string() argument".as_ptr());
        assert_eq!(evs_ret, OK);
        DID_ECHO_STRING_EMSG = false;
        if !len.is_null() {
            *len = ga.len();
        }
        xmemdupz(ga.as_ptr().cast(), ga.len()).cast::<c_char>()
    }
}

/// Return a string with the `:echo` representation of a variable (`encode_tv2echo`).
pub unsafe fn encode_tv2echo(tv: *mut typval_T, len: *mut usize) -> *mut c_char {
    unsafe {
        let mut ga: Vec<u8> = Vec::with_capacity(80);
        let tv_ref = &*tv;
        if tv_ref.v_type == VarType::VAR_STRING || tv_ref.v_type == VarType::VAR_FUNC {
            if !tv_ref.vval.v_string.is_null() {
                buf_concat_cstr(&mut ga, tv_ref.vval.v_string);
            }
        } else {
            let eve_ret = encode_vim_to_echo(&mut ga, tv, c":echo argument".as_ptr());
            assert_eq!(eve_ret, OK);
        }
        if !len.is_null() {
            *len = ga.len();
        }
        xmemdupz(ga.as_ptr().cast(), ga.len()).cast::<c_char>()
    }
}

/// Return a JSON string representation of a variable (`encode_tv2json`).
pub unsafe fn encode_tv2json(tv: *mut typval_T, len: *mut usize) -> *mut c_char {
    unsafe {
        let mut ga: Vec<u8> = Vec::with_capacity(80);
        let mut backend = EncodeBackend::Text {
            gap: &mut ga,
            mode: TextEncodeMode::Json,
        };
        let evj_ret =
            encode_vim_with_backend(&mut backend, tv, c"encode_tv2json() argument".as_ptr());
        DID_ECHO_STRING_EMSG = false;
        if evj_ret == FAIL {
            ga.clear();
        }
        if !len.is_null() {
            *len = ga.len();
        }
        xmemdupz(ga.as_ptr().cast(), ga.len()).cast::<c_char>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_from_list_substitutes_nl_with_nul_and_joins_with_nl() {
        let mut s1 = *b"a\nb\0";
        let mut s2 = *b"c\0";
        let mut li2 = listitem_T {
            li_next: std::ptr::null_mut(),
            li_prev: std::ptr::null_mut(),
            li_tv: typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_string: s2.as_mut_ptr().cast::<c_char>(),
                },
            },
        };
        let mut li1 = listitem_T {
            li_next: &mut li2,
            li_prev: std::ptr::null_mut(),
            li_tv: typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_string: s1.as_mut_ptr().cast::<c_char>(),
                },
            },
        };
        li2.li_prev = &mut li1;
        let list = list_T {
            lv_first: &mut li1,
            lv_last: &mut li2,
            lv_watch: Vec::new(),
            lv_idx_item: std::ptr::null_mut(),
            lv_copylist: std::ptr::null_mut(),
            lv_used_next: std::ptr::null_mut(),
            lv_used_prev: std::ptr::null_mut(),
            lv_refcount: 1,
            lv_len: 2,
            lv_idx: 0,
            lv_copyID: 0,
            lv_lock: 0,
            lua_table_ref: -2,
        };
        let mut len = 0usize;
        let mut buf: *mut c_char = std::ptr::null_mut();
        unsafe {
            assert!(encode_vim_list_to_buf(&list, &mut len, &mut buf));
            assert_eq!(len, 5);
            let slice = std::slice::from_raw_parts(buf.cast::<u8>(), len);
            assert_eq!(slice, b"a\0b\nc");
            xfree(buf.cast());
        }
        li1.li_tv.v_type = VarType::VAR_UNKNOWN;
        li2.li_tv.v_type = VarType::VAR_UNKNOWN;
    }

    #[test]
    fn encode_list_write_handles_nl_and_nul_sequences() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        use crate::eval::typval::{tv_list_alloc, tv_list_free};

        unsafe {
            let check = |chunks: &[&[u8]], expected: &[Option<&[u8]>]| {
                let l = tv_list_alloc(0);
                for &chunk in chunks {
                    encode_list_write(l, chunk.as_ptr().cast(), chunk.len());
                }
                assert_eq!((*l).lv_len as usize, expected.len());
                let mut li = (*l).lv_first;
                for &exp in expected {
                    assert!(!li.is_null());
                    assert_eq!((*li).li_tv.v_type, VarType::VAR_STRING);
                    let ptr = (*li).li_tv.vval.v_string;
                    match exp {
                        None => assert!(ptr.is_null()),
                        Some(bytes) => {
                            assert!(!ptr.is_null());
                            assert_eq!(CStr::from_ptr(ptr).to_bytes(), bytes);
                        }
                    }
                    li = (*li).li_next;
                }
                tv_list_free(l);
            };

            check(&[b""], &[]);
            check(&[b"abc"], &[Some(b"abc")]);
            check(&[b"\nabc"], &[None, Some(b"abc")]);
            check(&[b"\nabc", b"\nabc"], &[None, Some(b"abc"), Some(b"abc")]);
            check(&[b"abc\n"], &[Some(b"abc"), None]);
            check(&[b"abc\n", b"abc\n"], &[Some(b"abc"), Some(b"abc"), None]);
            check(
                &[b"\na\nb\n", b"\na\nb\n"],
                &[
                    None,
                    Some(b"a"),
                    Some(b"b"),
                    None,
                    Some(b"a"),
                    Some(b"b"),
                    None,
                ],
            );
            check(
                &[b"\0\n\0\n\0", b"\0\n\0\n\0"],
                &[
                    Some(b"\n"),
                    Some(b"\n"),
                    Some(b"\n\n"),
                    Some(b"\n"),
                    Some(b"\n"),
                ],
            );
            check(
                &[b"\n\0\n\0\n", b"\n\0\n\0\n"],
                &[
                    None,
                    Some(b"\n"),
                    Some(b"\n"),
                    None,
                    Some(b"\n"),
                    Some(b"\n"),
                    None,
                ],
            );
            check(&[b"\n", b"\n"], &[None, None, None]);
            check(
                &[b"\n\n\n", b"\n\n\n"],
                &[None, None, None, None, None, None, None],
            );
        }
    }
}
