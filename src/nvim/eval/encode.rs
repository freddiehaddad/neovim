//! Encoding Vimscript `TypVal` values to strings, `:echo`, JSON, and MessagePack
//! (`src/nvim/eval/encode.c`).

use crate::api::private::converter::{K_BOOL_VAR_TRUE, K_SPECIAL_VAR_NULL};
use crate::api::private::validate::NvimString;
use crate::buffer::NOTDONE;
use crate::eval::decode::{
    K_MP_EXT, K_MP_INTEGER, K_MP_MAP, SURROGATE_FIRST_CHAR, SURROGATE_HI_END, SURROGATE_HI_START,
    SURROGATE_LO_END, SURROGATE_LO_START,
};
use crate::eval::typval::{
    TypvalVval, VAR_UNLOCKED, VarType, TvBlob, TvDict, TvList, Listitem, TvPartial, tv_blob_get,
    tv_blob_len, tv_dict_find, tv_list_append_string, tv_list_first, tv_list_idx_of_item,
    tv_list_last, tv_list_len, TypVal,
};
use crate::eval::vars::{NUM_MSGPACK_TYPES, eval_msgpack_type_lists_mut};
use crate::eval::{get_copyID, partial_name};
use crate::hashtab::{Hashitem, hashitem_empty};
use crate::keycodes::{BS, CAR, NL, TAB};
use crate::math::{FAIL, OK};
use crate::mbyte::{utf_char2len, utf_printable, utf_ptr2char, utf_ptr2len};
use crate::memory::{cstr_from_bytes};
use crate::message::{emsg, internal_error, semsg};
use crate::msgpack_rpc::packer::{
    PackerBuffer, mpack_array, mpack_bin, mpack_bool, mpack_check_buffer, mpack_ext, mpack_float8,
    mpack_integer, mpack_map, mpack_nil, mpack_str, mpack_uint64,
};
use crate::regexp::NUMBUFLEN;
use crate::strings::{vim_snprintf, vim_snprintf_safelen};
use crate::testing::ga_concat_literal;
use std::ffi::{c_char, c_int};

pub(crate) const K_MP_NIL: usize = 0;
pub(crate) const K_MP_BOOLEAN: usize = 1;
pub(crate) const K_MP_FLOAT: usize = 3;
pub(crate) const K_MP_STRING: usize = 4;
pub(crate) const K_MP_ARRAY: usize = 5;

const FF: c_int = 0x0c;

pub const encode_bool_var_names: [*const c_char; 2] = [c"v:false".as_ptr(), c"v:true".as_ptr()];
pub const encode_special_var_names: [*const c_char; 1] = [c"v:null".as_ptr()];

struct EvalEncodeState {
    DID_ECHO_STRING_EMSG: bool,
}

impl EvalEncodeState {
    const fn new() -> Self {
        Self {
            DID_ECHO_STRING_EMSG: false,
        }
    }
}

static ENCODE_STATE: crate::globals::GlobalState<EvalEncodeState> = crate::globals::GlobalState::new(EvalEncodeState::new());

#[inline]
fn encode_st() -> &'static mut EvalEncodeState {
    unsafe { &mut *ENCODE_STATE.as_ptr() }
}

/// Structure defining state for `encode_read_from_list()` (`ListReaderState`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ListReaderState {
    pub li: *const Listitem,
    pub offset: usize,
    pub li_length: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MPContainerRef {
    Dict(*mut TvDict),
    List(*mut TvList),
    Pairs(*mut TvList),
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
        dict: *mut TvDict,
        dictp: *mut *mut TvDict,
        hi: *mut Hashitem,
        todo: usize,
    },
    List {
        list: *mut TvList,
        li: *mut Listitem,
    },
    Pairs {
        list: *mut TvList,
        li: *mut Listitem,
    },
    Partial {
        stage: MPConvPartialStage,
        pt: *mut TvPartial,
    },
    PartialList {
        arg: *mut TypVal,
        argv: *mut TypVal,
        todo: usize,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct MPConvStackVal {
    pub(crate) tv: *mut TypVal,
    pub(crate) saved_copy_id: c_int,
    pub(crate) data: MPConvStackData,
}

/// Msgpack callback for writing to `readfile()`-style list (`encode_list_write`).
pub fn encode_list_write(mut list: Option<&mut TvList>, buf: impl crate::strings::IntoCCharPtr, len: usize) { let buf = buf.into_c_char_ptr();
    unsafe {
        if len == 0 {
            return;
        }
        let end = buf.add(len);
        let mut line_end = buf;
        let li = tv_list_last(list.as_deref());

        if !li.is_null() {
            line_end = { let _a = (buf).cast::<u8>(); let _sz = len; let _off = if _sz == 0 { 0 } else { std::slice::from_raw_parts(_a, _sz).iter().position(|&_b| _b == (NL as c_char) as u8).unwrap_or(_sz) }; _a.add(_off).cast_mut().cast::<std::ffi::c_void>() }
                .cast_const()
                .cast::<c_char>();
            if line_end != buf {
                let line_length = (line_end as usize) - (buf as usize);
                let str_ptr = (*li).li_tv.vval.v_string();
                let li_len = if str_ptr.is_null() {
                    0
                } else {
                    crate::memory::cstr_len(str_ptr)
                };
                let mut combined = vec![0 as c_char; li_len + line_length];
                if !str_ptr.is_null() {
                    std::ptr::copy_nonoverlapping(str_ptr, combined.as_mut_ptr(), li_len);
                }
                let dst = combined.as_mut_ptr().add(li_len);
                std::ptr::copy_nonoverlapping(buf.cast::<u8>(), dst.cast::<u8>(), line_length);
                { let _d = (dst).cast::<u8>(); let _sz = line_length; if _sz > 0 { let _c = 0_u8; let _x = (NL as c_char) as u8; for _b in std::slice::from_raw_parts_mut(_d, _sz) { if *_b == _c { *_b = _x; } } } };
                crate::eval::typval::tv_set_string_len(
                    &mut (*li).li_tv,
                    combined.as_ptr(),
                    combined.len(),
                );
            }
            line_end = line_end.add(1);
        }

        while (line_end as usize) < (end as usize) {
            let line_start = line_end;
            line_end = { let _a = (line_start).cast::<u8>(); let _sz = (end as usize) - (line_start as usize); let _off = if _sz == 0 { 0 } else { std::slice::from_raw_parts(_a, _sz).iter().position(|&_b| _b == (NL as c_char) as u8).unwrap_or(_sz) }; _a.add(_off).cast_mut().cast::<std::ffi::c_void>() }
            .cast_const()
            .cast::<c_char>();
            if line_end != line_start {
                let line_length = (line_end as usize) - (line_start as usize);
                let mut line_buf = vec![0 as c_char; line_length];
                std::ptr::copy_nonoverlapping(
                    line_start.cast::<u8>(),
                    line_buf.as_mut_ptr().cast::<u8>(),
                    line_length,
                );
                { let _d = (line_buf.as_mut_ptr()).cast::<u8>(); let _sz = line_length; if _sz > 0 { let _c = 0_u8; let _x = (NL as c_char) as u8; for _b in std::slice::from_raw_parts_mut(_d, _sz) { if *_b == _c { *_b = _x; } } } };
                crate::eval::typval::tv_list_append_string(
                     list.as_deref_mut().unwrap(),
                    line_buf.as_ptr(),
                    line_length as isize,
                );
            } else {
                tv_list_append_string( list.as_deref_mut().unwrap(), std::ptr::null(), 0);
            }
            line_end = line_end.add(1);
        }
        if line_end == end {
            tv_list_append_string( list.unwrap(), std::ptr::null(), 0);
        }
    }
}

/// Initialize `ListReaderState` structure (`encode_init_lrstate`).
pub fn encode_init_lrstate(list: Option<&TvList>) -> ListReaderState {
    let first = tv_list_first(list);
    let li_length = if first.is_null() {
        0
    } else {
        let s = unsafe { (*first).li_tv.vval.v_string() };
        if s.is_null() {
            0
        } else {
             crate::memory::cstr_len(s)
        }
    };
    ListReaderState {
        li: first,
        offset: 0,
        li_length,
    }
}

/// Read bytes from list (`encode_read_from_list`).
pub fn encode_read_from_list(
    state: *mut ListReaderState,
    buf: *mut c_char,
    nbuf: usize,
    read_bytes: &mut usize,
) -> c_int {
    let state = unsafe { &mut *state };
    let buf_end = unsafe { buf.add(nbuf) };
    let mut p = buf;
    while (p as usize) < (buf_end as usize) {
        while state.offset < state.li_length && (p as usize) < (buf_end as usize) {
            let s = unsafe { (*state.li).li_tv.vval.v_string() };
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
                *read_bytes = (p as usize) - (buf as usize);
                return OK;
            }
            unsafe {
                *p = NL as c_char;
                p = p.add(1);
            }
            if unsafe { (*state.li).li_tv.v_type } != VarType::VarString {
                *read_bytes = (p as usize) - (buf as usize);
                return FAIL;
            }
            state.offset = 0;
            let s = unsafe { (*state.li).li_tv.vval.v_string() };
            state.li_length = if s.is_null() {
                0
            } else {
                 crate::memory::cstr_len(s)
            };
        }
    }
    *read_bytes = nbuf;
    if state.offset < state.li_length || unsafe { !(*state.li).li_next.is_null() } {
        NOTDONE
    } else {
        OK
    }
}

/// Convert `readfile()`-style list to a `Vec<c_char>` buffer (`encode_vim_list_to_buf`).
pub fn encode_vim_list_to_buf(list: Option<&TvList>) -> Option<Vec<c_char>> {
    let mut len = 0usize;
    for li in crate::eval::typval::list_iter_mut(tv_list_first(list)) {
        let tv =  { &li.li_tv };
        if tv.v_type != VarType::VarString {
            return None;
        }
        len += 1;
        let s = tv.vval.v_string();
        if !s.is_null() {
            len +=  crate::memory::cstr_len(s);
        }
    }
    len = len.saturating_sub(1);
    if len == 0 {
        return Some(Vec::new());
    }
    let mut lrstate = encode_init_lrstate(list);
    let mut buf = vec![0 as c_char; len];
    let mut read_bytes = 0usize;
    if encode_read_from_list(&mut lrstate, buf.as_mut_ptr(), len, &mut read_bytes) == FAIL
    {
        std::process::abort();
    }
    assert_eq!(len, read_bytes);
    Some(buf)
}

/// Check whether given key can be used in `json_encode()` (`encode_check_json_key`).
pub fn encode_check_json_key(tv: &TypVal) -> bool {
    unsafe {
        if tv.v_type == VarType::VarString {
            return true;
        }
        if tv.v_type != VarType::VarDict {
            return false;
        }
        let spdict = tv.vval.v_dict();
        if spdict.is_null() || (*spdict).dv_hashtab.ht_used != 2 {
            return false;
        }
        let type_di = tv_dict_find(spdict.as_ref(), c"_TYPE", 5);
        if type_di.is_null()
            || (*type_di).di_tv.v_type != VarType::VarList
            || (*type_di).di_tv.vval.v_list().cast_const() != (*eval_msgpack_type_lists_mut())[K_MP_STRING]
        {
            return false;
        }
        let val_di = tv_dict_find(spdict.as_ref(), c"_VAL", 4);
        if val_di.is_null() || (*val_di).di_tv.v_type != VarType::VarList {
            return false;
        }
        let val_list = (*val_di).di_tv.vval.v_list();
        if val_list.is_null() {
            return true;
        }
        for li in crate::eval::typval::list_iter_mut(tv_list_first(val_list.as_ref())) {
            if li.li_tv.v_type != VarType::VarString {
                return false;
            }
        }
        true
    }
}

#[inline]
fn buf_concat_cstr(gap: &mut Vec<u8>, s: *const c_char) {
     {
        if !s.is_null() {
            gap.extend_from_slice(crate::memory::cstr_as_bytes(s));
        }
    }
}

#[inline]
fn buf_concat_len(gap: &mut Vec<u8>, s: *const c_char, len: usize) {
     {
        if !s.is_null() && len > 0 {
            gap.extend_from_slice(crate::memory::cstr_slice(s, len));
        }
    }
}

fn conv_error(
    msg: &std::ffi::CStr,
    mpstack: &[MPConvStackVal],
    objname: impl crate::strings::IntoCCharPtr,
) -> c_int { let objname = objname.into_c_char_ptr();
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
                        (*dict).dv_hashtab.array_ptr()
                    } else {
                        hi.sub(1)
                    };
                    let mut key_tv = TypVal {
                        v_type: VarType::VarString,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::String((&*hi_ptr).key_ptr()),
                    };
                    let key = encode_tv2string(&mut key_tv, None);
                    vim_snprintf!(
                        iobuf.as_mut_ptr(),
                        iobuf.len(),
                        c"key %s",
                        key.as_ptr(),
                    );
                    buf_concat_cstr(&mut msg_ga, iobuf.as_ptr());
                }
                MPConvStackData::List { list, li } | MPConvStackData::Pairs { list, li } => {
                    let is_list = matches!(v.data, MPConvStackData::List { .. });
                    let first = tv_list_first(list.as_ref());
                    let idx = if li == first {
                        0
                    } else if li.is_null() {
                        tv_list_len(list.as_ref()) - 1
                    } else {
                        tv_list_idx_of_item(list.as_ref(), (*li).li_prev)
                    };
                    let prev_li: *const Listitem = if li.is_null() {
                        tv_list_last(list.as_ref())
                    } else {
                        (*li).li_prev
                    };
                    if is_list
                        || prev_li.is_null()
                        || ((*prev_li).li_tv.v_type != VarType::VarList
                            && tv_list_len(((*prev_li).li_tv.vval.v_list()).as_ref()) <= 0)
                    {
                        vim_snprintf!(iobuf.as_mut_ptr(), iobuf.len(), c"index %i", idx);
                        buf_concat_cstr(&mut msg_ga, iobuf.as_ptr());
                    } else {
                        let first_item = tv_list_first(((*prev_li).li_tv.vval.v_list()).as_ref());
                        assert!(!first_item.is_null());
                        let mut key_tv = (*first_item).li_tv;
                        let key = encode_tv2echo(&mut key_tv, None);
                        vim_snprintf!(
                            iobuf.as_mut_ptr(),
                            iobuf.len(),
                            c"key %s at index %i from special map",
                            key.as_ptr(),
                            idx,
                        );
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
                        c"argument %i",
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
        semsg!(msg.as_ptr(), objname, path_ptr);
        FAIL
    }
}

fn convert_to_single_quoted_string(gap: &mut Vec<u8>, buf: impl crate::strings::IntoCCharPtr, len: usize) { let buf = buf.into_c_char_ptr();
    unsafe {
        if buf.is_null() {
            ga_concat_literal(gap, b"''");
        } else {
            let quotes = { let _d = (buf).cast::<u8>(); let _sz = len; if _sz == 0 { 0 } else { std::slice::from_raw_parts(_d, _sz).iter().filter(|&&_b| _b == (b'\'' as c_char) as u8).count() } };
            gap.reserve(2 + len + quotes);
            gap.push(b'\'');
            let bytes = crate::memory::cstr_slice(buf, len);
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

fn convert_to_json_string(gap: &mut Vec<u8>, buf: impl crate::strings::IntoCCharPtr, len: usize) -> bool { let buf = buf.into_c_char_ptr();
    const XDIGITS: &[u8; 16] = b"0123456789ABCDEF";

    unsafe {
        if buf.is_null() {
            ga_concat_literal(gap, b"\"\"");
            return true;
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
                            c"E474: String \"%.*s\" contains byte that does not start any UTF-8 character",
                            (len - start) as c_int,
                            buf.add(start),
                        );
                        return false;
                    } else if (SURROGATE_HI_START..=SURROGATE_HI_END).contains(&ch)
                        || (SURROGATE_LO_START..=SURROGATE_LO_END).contains(&ch)
                    {
                        let start = i - shift;
                        semsg!(
                            c"E474: UTF-8 string contains code point which belongs to a surrogate pair: %.*s",
                            (len - start) as c_int,
                            buf.add(start),
                        );
                        return false;
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
        true
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
    fn convert_one_value_err_name(&self) -> &'static std::ffi::CStr {
        match self {
            Self::Text {
                mode: TextEncodeMode::String,
                ..
            } => c"_typval_encode_string_convert_one_value()",
            Self::Text {
                mode: TextEncodeMode::Echo,
                ..
            } => c"_typval_encode_echo_convert_one_value()",
            Self::Text {
                mode: TextEncodeMode::Json,
                ..
            } => c"_typval_encode_json_convert_one_value()",
            Self::Msgpack { .. } => c"_typval_encode_msgpack_convert_one_value()",
        }
    }

    #[inline]
    fn check_before(&mut self) {
        if let Self::Msgpack { packer } = *self {
            unsafe {
                mpack_check_buffer(&mut *packer);
            }
        }
    }

    #[inline]
    fn conv_nil(&mut self) {
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
    fn conv_bool(&mut self, val: bool) {
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
    fn conv_number(&mut self, num: i64) {
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
    fn conv_unsigned_number(&mut self, num: u64) {
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
    fn conv_float(&mut self, flt: f64) -> bool {
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
                            c"%g",
                            flt,
                        );
                        buf_concat_len(gap, numbuf.as_ptr(), len);
                    }
                    true
                }
                Self::Text {
                    gap,
                    mode: TextEncodeMode::Json,
                } => {
                    if flt.is_nan() {
                        emsg(c"E474: Unable to represent NaN value in JSON");
                        return false;
                    } else if flt.is_infinite() {
                        emsg(c"E474: Unable to represent infinity in JSON");
                        return false;
                    } else {
                        let mut numbuf = [0 as c_char; NUMBUFLEN];
                        let len = vim_snprintf_safelen!(
                            numbuf.as_mut_ptr(),
                            numbuf.len(),
                            c"%g",
                            flt,
                        );
                        buf_concat_len(gap, numbuf.as_ptr(), len);
                    }
                    true
                }
                Self::Msgpack { packer } => {
                    mpack_float8(&mut (**packer).ptr, flt);
                    true
                }
            }
        }
    }

    #[inline]
    fn conv_string(&mut self, buf: impl crate::strings::IntoCCharPtr, len: usize) -> bool { let buf = buf.into_c_char_ptr();
        unsafe {
            match self {
                Self::Text {
                    gap,
                    mode: TextEncodeMode::String | TextEncodeMode::Echo,
                } => {
                    convert_to_single_quoted_string(gap, buf, len);
                    true
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
                        &mut **packer,
                    );
                    true
                }
            }
        }
    }

    #[inline]
    fn conv_str_string(&mut self, buf: impl crate::strings::IntoCCharPtr, len: usize) -> bool { let buf = buf.into_c_char_ptr();
        unsafe {
            match self {
                Self::Text { .. } => self.conv_string(buf, len),
                Self::Msgpack { packer } => {
                    mpack_str(
                        NvimString {
                            data: buf.cast_mut(),
                            size: len,
                        },
                        &mut **packer,
                    );
                    true
                }
            }
        }
    }

    #[inline]
    fn conv_ext_string(&mut self, buf: impl crate::strings::IntoCCharPtr, len: usize, ext_type: i8) -> bool { let buf = buf.into_c_char_ptr();
        unsafe {
            match self {
                Self::Text {
                    mode: TextEncodeMode::Json,
                    ..
                } => {
                    emsg(c"E474: Unable to convert EXT string to JSON");
                    false
                }
                Self::Text { .. } => true,
                Self::Msgpack { packer } => {
                    mpack_ext(buf, len, ext_type, &mut **packer);
                    true
                }
            }
        }
    }

    #[inline]
    fn conv_blob(&mut self, blob: Option<&TvBlob>, len: c_int) {
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
                                c"%02X",
                                tv_blob_get( blob.unwrap(), i) as c_int,
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
                                c"%d",
                                tv_blob_get( blob.unwrap(), i) as c_int,
                            );
                            buf_concat_len(gap, numbuf.as_ptr(), nlen);
                        }
                        gap.push(b']');
                    }
                }
                Self::Msgpack { packer } => {
                    let data = match blob {
                        Some(b) if !b.bv_ga.is_empty() => b.bv_ga.as_ptr().cast_mut().cast::<c_char>(),
                        _ => std::ptr::null_mut(),
                    };
                    mpack_bin(
                        NvimString {
                            data,
                            size: len as usize,
                        },
                        &mut **packer,
                    );
                }
            }
        }
    }

    #[inline]
    fn conv_func_start(
        &mut self,
        fun: impl crate::strings::IntoCCharPtr,
        prefix: &[u8],
        mpstack: &[MPConvStackVal],
        objname: impl crate::strings::IntoCCharPtr,
    ) -> c_int { let fun = fun.into_c_char_ptr(); let objname = objname.into_c_char_ptr();
        unsafe {
            match self {
                Self::Text {
                    gap,
                    mode: TextEncodeMode::String | TextEncodeMode::Echo,
                } => {
                    if fun.is_null() {
                        internal_error(c"string(): NULL function name");
                        ga_concat_literal(gap, b"function(NULL");
                    } else {
                        ga_concat_literal(gap, b"function(");
                        let name_off = gap.len();
                        ga_concat_literal(gap, prefix);
                        let fun_len = crate::memory::cstr_len(fun);
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
                    c"E474: Error while dumping %s, %s: attempt to dump function reference",
                    mpstack,
                    objname,
                ),
                Self::Msgpack { .. } => conv_error(
                    c"E5004: Error while dumping %s, %s: attempt to dump function reference",
                    mpstack,
                    objname,
                ),
            }
        }
    }

    #[inline]
    fn conv_func_before_args(&mut self, len: c_int) {
        if let Self::Text { gap, .. } = self
            && len != 0
        {
            ga_concat_literal(gap, b", ");
        }
    }

    #[inline]
    fn conv_func_before_self(&mut self, len: isize) {
        if let Self::Text { gap, .. } = self
            && len != -1
        {
            ga_concat_literal(gap, b", ");
        }
    }

    #[inline]
    fn conv_func_end(&mut self) {
        if let Self::Text { gap, .. } = self {
            gap.push(b')');
        }
    }

    #[inline]
    fn conv_empty_list(&mut self) {
        unsafe {
            match self {
                Self::Text { gap, .. } => ga_concat_literal(gap, b"[]"),
                Self::Msgpack { packer } => mpack_array(&mut (**packer).ptr, 0),
            }
        }
    }

    #[inline]
    fn conv_list_start(&mut self, len: c_int) {
        unsafe {
            match self {
                Self::Text { gap, .. } => gap.push(b'['),
                Self::Msgpack { packer } => mpack_array(&mut (**packer).ptr, len as u32),
            }
        }
    }

    #[inline]
    fn conv_list_between_items(&mut self) {
        if let Self::Text { gap, .. } = self {
            ga_concat_literal(gap, b", ");
        }
    }

    #[inline]
    fn conv_list_end(&mut self) {
        if let Self::Text { gap, .. } = self {
            gap.push(b']');
        }
    }

    #[inline]
    fn conv_empty_dict(&mut self) {
        unsafe {
            match self {
                Self::Text { gap, .. } => ga_concat_literal(gap, b"{}"),
                Self::Msgpack { packer } => mpack_map(&mut (**packer).ptr, 0),
            }
        }
    }

    #[inline]
    fn conv_dict_start(&mut self, len: usize) {
        unsafe {
            match self {
                Self::Text { gap, .. } => gap.push(b'{'),
                Self::Msgpack { packer } => mpack_map(&mut (**packer).ptr, len as u32),
            }
        }
    }

    #[inline]
    fn conv_dict_after_key(&mut self) {
        if let Self::Text { gap, .. } = self {
            ga_concat_literal(gap, b": ");
        }
    }

    #[inline]
    fn conv_dict_between_items(&mut self) {
        if let Self::Text { gap, .. } = self {
            ga_concat_literal(gap, b", ");
        }
    }

    #[inline]
    fn conv_dict_end(&mut self) {
        if let Self::Text { gap, .. } = self {
            gap.push(b'}');
        }
    }

    #[inline]
    fn special_dict_key_check(&mut self, key: &TypVal) -> bool {
        if matches!(
            *self,
            Self::Text {
                mode: TextEncodeMode::Json,
                ..
            }
        ) && !encode_check_json_key(key)
        {
            emsg(c"E474: Invalid key in special dictionary");
            return false;
        }
        true
    }

    fn check_self_reference(
        &mut self,
        container: MPContainerRef,
        val_copy_id: &mut c_int,
        mpstack: &[MPConvStackVal],
        copy_id: c_int,
        objname: impl crate::strings::IntoCCharPtr,
    ) -> c_int { let objname = objname.into_c_char_ptr();
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
                    if !encode_st().DID_ECHO_STRING_EMSG {
                        encode_st().DID_ECHO_STRING_EMSG = true;
                        emsg(
                            c"E724: unable to correctly dump variable with self-referencing container",
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
                    if !encode_st().DID_ECHO_STRING_EMSG {
                        encode_st().DID_ECHO_STRING_EMSG = true;
                        emsg(
                            c"E724: unable to correctly dump variable with self-referencing container",
                        );
                    }
                    OK
                }
                Self::Msgpack { .. } => conv_error(
                    c"E5005: Unable to dump %s: container references itself in %s",
                    mpstack,
                    objname,
                ),
            }
    }
}

fn convert_one_value(
    backend: &mut EncodeBackend,
    mpstack: &mut Vec<MPConvStackVal>,
    tv: *mut TypVal,
    copy_id: c_int,
    objname: *const c_char,
) -> c_int {
    unsafe {
        backend.check_before();
        let tv_ref = &mut *tv;
        match tv_ref.v_type {
            VarType::VarString => {
                let s = tv_ref.vval.v_string();
                let len = if s.is_null() {
                    0
                } else {
                    crate::memory::cstr_len(s)
                };
                if !backend.conv_string(s, len) {
                    return FAIL;
                }
            }
            VarType::VarNumber => {
                backend.conv_number(tv_ref.vval.v_number());
            }
            VarType::VarFloat => {
                if !backend.conv_float(tv_ref.vval.v_float()) {
                    return FAIL;
                }
            }
            VarType::VarBlob => {
                let blob = tv_ref.vval.v_blob();
                backend.conv_blob(blob.as_ref(), tv_blob_len(blob.as_ref()));
            }
            VarType::VarFunc => {
                if backend.conv_func_start(tv_ref.vval.v_string(), b"", mpstack, objname) == FAIL {
                    return FAIL;
                }
                backend.conv_func_before_args(0);
                backend.conv_func_before_self(-1);
                backend.conv_func_end();
            }
            VarType::VarPartial => {
                let pt = tv_ref.vval.v_partial();
                let fun = if pt.is_null() {
                    std::ptr::null_mut()
                } else {
                    partial_name(pt.as_ref())
                };
                let prefix: &[u8] = if !fun.is_null()
                    && !pt.is_null()
                    && (*pt).pt_name.is_none()
                    && ((crate::memory::cstr_first_byte(fun)).is_ascii_uppercase())
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
            VarType::VarList => {
                let list = tv_ref.vval.v_list();
                if list.is_null() || tv_list_len(list.as_ref()) == 0 {
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
                    backend.conv_list_start(tv_list_len(list.as_ref()));
                    mpstack.push(MPConvStackVal {
                        tv,
                        saved_copy_id,
                        data: MPConvStackData::List {
                            list,
                            li: tv_list_first(list.as_ref()),
                        },
                    });
                }
            }
            VarType::VarBool => {
                backend.conv_bool(tv_ref.vval.v_bool() == K_BOOL_VAR_TRUE as c_int);
            }
            VarType::VarSpecial => {
                if tv_ref.vval.v_special() == K_SPECIAL_VAR_NULL as c_int {
                    backend.conv_nil();
                }
            }
            VarType::VarDict => {
                let dict = tv_ref.vval.v_dict();
                if dict.is_null() || (*dict).dv_hashtab.ht_used == 0 {
                    backend.conv_empty_dict();
                    return OK;
                }
                let mut handled_special = false;
                if backend.allow_specials() && (*dict).dv_hashtab.ht_used == 2 {
                    let type_di = tv_dict_find(dict.as_ref(), c"_TYPE", 5);
                    if !type_di.is_null() && (*type_di).di_tv.v_type == VarType::VarList {
                        let val_di = tv_dict_find(dict.as_ref(), c"_VAL", 4);
                        if !val_di.is_null() {
                            let mut mp_idx = NUM_MSGPACK_TYPES;
                            for (i, tl) in { *eval_msgpack_type_lists_mut() }.into_iter().enumerate() {
                                if (*type_di).di_tv.vval.v_list().cast_const() == tl {
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
                                        if (*val_di).di_tv.v_type == VarType::VarNumber {
                                            backend.conv_bool((*val_di).di_tv.vval.v_number() != 0);
                                            handled_special = true;
                                        }
                                    }
                                    K_MP_INTEGER => {
                                        if (*val_di).di_tv.v_type == VarType::VarList {
                                            let val_list = (*val_di).di_tv.vval.v_list();
                                            if tv_list_len(val_list.as_ref()) == 4 {
                                                let sign_li = tv_list_first(val_list.as_ref());
                                                let highest_li = (*sign_li).li_next;
                                                let high_li = (*highest_li).li_next;
                                                let low_li = tv_list_last(val_list.as_ref());
                                                if (*sign_li).li_tv.v_type == VarType::VarNumber
                                                    && (*sign_li).li_tv.vval.v_number() != 0
                                                    && (*highest_li).li_tv.v_type
                                                        == VarType::VarNumber
                                                    && (*highest_li).li_tv.vval.v_number() >= 0
                                                    && (*high_li).li_tv.v_type
                                                        == VarType::VarNumber
                                                    && (*high_li).li_tv.vval.v_number() >= 0
                                                    && (*low_li).li_tv.v_type == VarType::VarNumber
                                                    && (*low_li).li_tv.vval.v_number() >= 0
                                                {
                                                    let sign = (*sign_li).li_tv.vval.v_number();
                                                    let highest =
                                                        (*highest_li).li_tv.vval.v_number() as u64;
                                                    let high =
                                                        (*high_li).li_tv.vval.v_number() as u64;
                                                    let low = (*low_li).li_tv.vval.v_number() as u64;
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
                                        if (*val_di).di_tv.v_type == VarType::VarFloat {
                                            if !backend.conv_float((*val_di).di_tv.vval.v_float())
                                            {
                                                return FAIL;
                                            }
                                            handled_special = true;
                                        }
                                    }
                                    K_MP_STRING => {
                                        if (*val_di).di_tv.v_type == VarType::VarList
                                            && let Some(buf) =
                                                encode_vim_list_to_buf(((*val_di).di_tv.vval.v_list()).as_ref())
                                        {
                                            if !backend.conv_str_string(buf.as_ptr(), buf.len()) {
                                                return FAIL;
                                            }
                                            handled_special = true;
                                        }
                                    }
                                    K_MP_ARRAY => {
                                        if (*val_di).di_tv.v_type == VarType::VarList {
                                            let val_list = (*val_di).di_tv.vval.v_list();
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
                                            backend.conv_list_start(tv_list_len(val_list.as_ref()));
                                            mpstack.push(MPConvStackVal {
                                                tv,
                                                saved_copy_id,
                                                data: MPConvStackData::List {
                                                    list: val_list,
                                                    li: tv_list_first(val_list.as_ref()),
                                                },
                                            });
                                            handled_special = true;
                                        }
                                    }
                                    K_MP_MAP => {
                                        if (*val_di).di_tv.v_type == VarType::VarList {
                                            let val_list = (*val_di).di_tv.vval.v_list();
                                            if val_list.is_null() || tv_list_len(val_list.as_ref()) == 0 {
                                                backend.conv_empty_dict();
                                                handled_special = true;
                                            } else {
                                                let mut all_pairs = true;
                                                let mut li = tv_list_first(val_list.as_ref());
                                                while !li.is_null() {
                                                    if (*li).li_tv.v_type != VarType::VarList
                                                        || tv_list_len(((*li).li_tv.vval.v_list()).as_ref()) != 2
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
                                                        tv_list_len(val_list.as_ref()) as usize
                                                    );
                                                    mpstack.push(MPConvStackVal {
                                                        tv,
                                                        saved_copy_id,
                                                        data: MPConvStackData::Pairs {
                                                            list: val_list,
                                                            li: tv_list_first(val_list.as_ref()),
                                                        },
                                                    });
                                                    handled_special = true;
                                                }
                                            }
                                        }
                                    }
                                    K_MP_EXT if (*val_di).di_tv.v_type == VarType::VarList => {
                                        let val_list = (*val_di).di_tv.vval.v_list();
                                        if tv_list_len(val_list.as_ref()) == 2 {
                                            let first = tv_list_first(val_list.as_ref());
                                            let last = tv_list_last(val_list.as_ref());
                                            if (*first).li_tv.v_type == VarType::VarNumber
                                                && (i8::MIN as i64..=i8::MAX as i64)
                                                    .contains(&(*first).li_tv.vval.v_number())
                                                && (*last).li_tv.v_type == VarType::VarList
                                                && let Some(mut buf) = encode_vim_list_to_buf(
                                                    ((*last).li_tv.vval.v_list()).as_ref(),
                                                )
                                            {
                                                let ext_type = (*first).li_tv.vval.v_number() as i8;
                                                if !backend.conv_ext_string(
                                                    buf.as_mut_ptr(),
                                                    buf.len(),
                                                    ext_type,
                                                ) {
                                                    return FAIL;
                                                }
                                                handled_special = true;
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
                            dictp: tv_ref.vval.v_dict_mut(),
                            hi: (*dict).dv_hashtab.array_ptr(),
                            todo: (*dict).dv_hashtab.ht_used,
                        },
                    });
                }
            }
            VarType::VarUnknown => {
                internal_error(backend.convert_one_value_err_name());
                return FAIL;
            }
        }
        OK
    }
}

fn encode_vim_with_backend(
    backend: &mut EncodeBackend,top_tv: &mut TypVal,
    objname: impl crate::strings::IntoCCharPtr,
) -> bool {
    let objname = objname.into_c_char_ptr();
    unsafe {
        let copy_id = get_copyID();
        let mut mpstack: Vec<MPConvStackVal> = Vec::with_capacity(8);

        if convert_one_value(backend, &mut mpstack, top_tv, copy_id, objname) == FAIL {
            return false;
        }

        while !mpstack.is_empty() {
            let cur_idx = mpstack.len() - 1;
            let saved_copy_id = mpstack[cur_idx].saved_copy_id;
            let next_tv: *mut TypVal = match mpstack[cur_idx].data {
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
                    let key_len = crate::memory::cstr_len(key_ptr);
                    if !backend.conv_str_string(key_ptr, key_len) {
                        return false;
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
                    } else if cur_li != tv_list_first(list.as_ref()) {
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
                    } else if li != tv_list_first(list.as_ref()) {
                        backend.conv_dict_between_items();
                    }
                    let kv_pair = (*li).li_tv.vval.v_list();
                    let first_item = tv_list_first(kv_pair.as_ref());
                    if !backend.special_dict_key_check(&(*first_item).li_tv) {
                        return false;
                    }
                    if convert_one_value(
                        backend,
                        &mut mpstack,
                        &mut (*first_item).li_tv,
                        copy_id,
                        objname,
                    ) == FAIL
                    {
                        return false;
                    }
                    backend.conv_dict_after_key();
                    let last_item = tv_list_last(kv_pair.as_ref());
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
                                        return false;
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
                                        hi: (*dict).dv_hashtab.array_ptr(),
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
                return false;
            }
        }

        true
    }
}

/// Convert Vimscript value to `:echo` output (`encode_vim_to_echo`).
pub fn encode_vim_to_echo(
    gap: &mut Vec<u8>,tv: &mut TypVal,
    objname: &std::ffi::CStr,
) -> bool {
    let mut backend = EncodeBackend::Text {
            gap,
            mode: TextEncodeMode::Echo,
        };
        encode_vim_with_backend(&mut backend, tv, objname.as_ptr())
}

/// Convert Vimscript value to msgpack string (`encode_vim_to_msgpack`).
pub fn encode_vim_to_msgpack(
    packer: &mut PackerBuffer,tv: &mut TypVal,
    objname: impl crate::strings::IntoCCharPtr,
) -> bool { let objname = objname.into_c_char_ptr();
    let mut backend = EncodeBackend::Msgpack { packer };
        encode_vim_with_backend(&mut backend, tv, objname)
}

/// Return a string with the string representation of a variable (`encode_tv2string`).
pub fn encode_tv2string(tv: &mut TypVal,mut len: Option<&mut usize>) -> std::ffi::CString {
     {
        let mut ga: Vec<u8> = Vec::with_capacity(80);
        let mut backend = EncodeBackend::Text {
            gap: &mut ga,
            mode: TextEncodeMode::String,
        };
        let evs_ret =
            encode_vim_with_backend(&mut backend, tv, c"encode_tv2string() argument");
        assert!(evs_ret);
        encode_st().DID_ECHO_STRING_EMSG = false;
        if let Some(ref mut p) = len {
            **p = ga.len();
        }
        cstr_from_bytes(&ga)
    }
}

/// Return a string with the `:echo` representation of a variable (`encode_tv2echo`).
pub fn encode_tv2echo(tv: &mut TypVal, len: Option<&mut usize>) -> std::ffi::CString {
     {
        let mut ga: Vec<u8> = Vec::with_capacity(80);
        if tv.v_type == VarType::VarString || tv.v_type == VarType::VarFunc {
            if !tv.vval.v_string().is_null() {
                buf_concat_cstr(&mut ga, tv.vval.v_string());
            }
        } else {
            let eve_ret = encode_vim_to_echo(&mut ga, tv, c":echo argument");
            assert!(eve_ret);
        }
        if let Some(len) = len {
            *len = ga.len();
        }
        cstr_from_bytes(&ga)
    }
}

/// Return a JSON string representation of a variable (`encode_tv2json`).
pub fn encode_tv2json(tv: &mut TypVal,mut len: Option<&mut usize>) -> std::ffi::CString {
     {
        let mut ga: Vec<u8> = Vec::with_capacity(80);
        let mut backend = EncodeBackend::Text {
            gap: &mut ga,
            mode: TextEncodeMode::Json,
        };
        let evj_ret =
            encode_vim_with_backend(&mut backend, tv, c"encode_tv2json() argument");
        encode_st().DID_ECHO_STRING_EMSG = false;
        if !evj_ret {
            ga.clear();
        }
        if let Some(ref mut p) = len {
            **p = ga.len();
        }
        cstr_from_bytes(&ga)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_from_list_substitutes_nl_with_nul_and_joins_with_nl() {
        let mut s1 = *b"a\nb\0";
        let mut s2 = *b"c\0";
        let mut li2 = Listitem {
            li_next: std::ptr::null_mut(),
            li_prev: std::ptr::null_mut(),
            li_tv: TypVal {
                v_type: VarType::VarString,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::String(s2.as_mut_ptr().cast::<c_char>()),
            },
        };
        let mut li1 = Listitem {
            li_next: &mut li2,
            li_prev: std::ptr::null_mut(),
            li_tv: TypVal {
                v_type: VarType::VarString,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::String(s1.as_mut_ptr().cast::<c_char>()),
            },
        };
        li2.li_prev = &mut li1;
        let list = TvList {
            lv_first: &mut li1,
            lv_last: &mut li2,
            lv_watch: Vec::new(),
            lv_idx_item: std::ptr::null_mut(),
            lv_copylist: std::ptr::null_mut(),
            lv_gc_idx: None,
            lv_refcount: 1,
            lv_len: 2,
            lv_idx: 0,
            lv_copyID: 0,
            lv_lock: 0,
            lua_table_ref: -2,
        };
         {
            let buf = encode_vim_list_to_buf(Some(&list)).unwrap();
            assert_eq!(buf.len(), 5);
            let slice = crate::memory::cstr_slice(buf.as_ptr(), buf.len());
            assert_eq!(slice, b"a\0b\nc");
        }
        li1.li_tv.v_type = VarType::VarUnknown;
        li2.li_tv.v_type = VarType::VarUnknown;
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
                    encode_list_write(l.as_mut(), chunk.as_ptr().cast(), chunk.len());
                }
                assert_eq!((*l).lv_len as usize, expected.len());
                let mut li = (*l).lv_first;
                for &exp in expected {
                    assert!(!li.is_null());
                    assert_eq!((*li).li_tv.v_type, VarType::VarString);
                    let ptr = (*li).li_tv.vval.v_string();
                    match exp {
                        None => assert!(ptr.is_null()),
                        Some(bytes) => {
                            assert!(!ptr.is_null());
                            assert_eq!(crate::memory::cstr_as_bytes(ptr), bytes);
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
