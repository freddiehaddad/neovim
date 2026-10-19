//! Code for mappings and abbreviations (`src/nvim/mapping.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_imports,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments,
    clippy::needless_range_loop
)]

#[allow(unused_imports)]
use crate::api::private::helpers::api_set_error;
#[allow(unused_imports)]
use crate::api::private::helpers::{
    api_clear_error, api_free_object, api_set_sctx, arena_dict, arena_take_arraybuilder,
    find_buffer_by_handle, string_to_cstr,
};
#[allow(unused_imports)]
use crate::cmdexpand::cmdline_fuzzy_complete;
use crate::errors::*;
#[allow(unused_imports)]
use crate::eval::typval::{
    tv_check_for_dict_arg, tv_dict_get_string, tv_get_string, tv_get_string_buf,
    tv_get_string_buf_chk,
};
#[allow(unused_imports)]
use crate::eval::typval::{
    tv_dict_alloc_ret, tv_dict_find, tv_list_alloc_ret, tv_list_append_dict,
};
#[allow(unused_imports)]
use crate::eval::typval::{tv_dict_get_bool, tv_dict_get_number, tv_get_bool};
#[allow(unused_imports)]
use crate::eval::userfunc::find_func;
#[allow(unused_imports)]
use crate::eval::vars::set_vim_var_char;
#[allow(unused_imports)]
use crate::eval::{eval_to_string, last_set_msg};
#[allow(unused_imports)]
use crate::ex_cmds::check_secure;
use crate::globals::*;
#[allow(unused_imports)]
use crate::input::{ins_typebuf, noremap_keys};
#[allow(unused_imports)]
use crate::lua::executor::nlua_call_ref;
#[allow(unused_imports)]
use crate::lua::executor::nlua_funcref_str;
#[allow(unused_imports)]
use crate::lua::executor::{api_free_luaref, api_new_luaref, nlua_set_sctx};
#[allow(unused_imports)]
use crate::message::{
    emsg, iemsg, message_filtered, msg, msg_clr_eos, msg_ext_set_kind, msg_outtrans,
    msg_outtrans_special, msg_putchar, msg_puts, msg_puts_hl, msg_start, str2special_arena,
    str2special_save,
};
use crate::message::{semsg, semsg_multiline, swmsg};
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::regexp::vim_regexec;
use crate::runtime::exestack;
#[allow(unused_imports)]
use crate::strings::vim_snprintf;
#[allow(unused_imports)]
use crate::strings::{sort_strings, vim_strchr};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::api::autocmd::{ArrayBuilder, Buffer};
use crate::api::buffer::KeyDict_keymap;
use crate::api::command::{cmdidx_T, exarg_T};
use crate::api::options::{CharBuf, optset_T};
use crate::api::private::converter::{object_to_vim_take_luaref, sctx_T, ufunc_T};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType,
};
use crate::api::tabpage::win_T;
use crate::charset::{skipwhite, transchar, vim_iswordp};
use crate::clipboard::{TypvalVval, VarType, list_T, typval_T};
use crate::eval::decode::{dict_T, dictitem_T};
use crate::eval::deprecated::{EvalFuncData, buf_T};
use crate::ex_session::{FILE, put_eol};
use crate::fuzzy::{FUZZY_SCORE_NONE, fuzmatch_str_T, fuzzy_match_str, fuzzymatches_to_strmatches};
use crate::garray::{
    ga_append, ga_append_via_ptr, ga_clear, ga_concat, ga_grow, ga_init, garray_T,
};
use crate::keycodes::{
    K_SPECIAL, KE_FILLER, KE_SNR, KS_EXTRA, KS_MODIFIER, KS_SPECIAL, KS_ZERO, REPTERM_DO_LT,
    REPTERM_FROM_PART, REPTERM_NO_SIMPLIFY, get_special_key_name, replace_termcodes,
    vim_strsave_escape_ks, vim_unescape_ks,
};
use crate::lua::converter::LuaRef;
use crate::map_glyph_cache::NvimString;
use crate::math::{FAIL, OK};
use crate::mbyte::{
    mb_prevptr, mb_unescape, utf_char2bytes, utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::memory::{
    Arena, arena_alloc, arena_finish, arena_mem_free, xcalloc, xfree, xmemcpyz, xstrdup, xstrlcpy,
};
use crate::msgpack_rpc::packer::Integer;
use crate::option::opt_error;
use crate::profile::estack_T;
use crate::sign::expand_T;

pub type scid_T = c_int;
pub type linenr_T = i32;
pub type TriState = c_int;

pub const MAXMAPLEN: usize = 50;
pub const MAX_MAPHASH: usize = 256;
const NUMBUFLEN: usize = 65;
const MB_MAXBYTES: usize = 31;

const NUL: c_char = 0;
const NL: c_int = b'\n' as c_int;
const Ctrl_C: c_char = 3;
const Ctrl_H: u8 = 8;
const Ctrl_J: c_int = 10;
const Ctrl_V: c_int = 22;
const Ctrl_RSB: c_int = 29;
const UCHAR_MAX: c_int = 255;

pub const MAPTYPE_MAP: c_int = 0;
pub const MAPTYPE_UNMAP: c_int = 1;
pub const MAPTYPE_NOREMAP: c_int = 2;
pub const MAPTYPE_UNMAP_LHS: c_int = 3;

const REMAP_YES: c_int = 0;
const REMAP_NONE: c_int = -1;
const REMAP_SCRIPT: c_int = -2;

const MODE_NORMAL: c_int = 0x01;
const MODE_VISUAL: c_int = 0x02;
const MODE_OP_PENDING: c_int = 0x04;
const MODE_CMDLINE: c_int = 0x08;
const MODE_INSERT: c_int = 0x10;
const MODE_LANGMAP: c_int = 0x20;
const MODE_SELECT: c_int = 0x40;
const MODE_TERMINAL: c_int = 0x80;

const LUA_NOREF: LuaRef = -2;
const HLF_8: c_int = 1;
const kCpoBslash: c_int = b'B' as c_int;

const KE_LUA: u8 = 103;
const ABBR_OFF: c_int = 0x100;
const K_ZERO: c_int = -((KS_ZERO as c_int) + ((KE_FILLER as c_int) << 8));

const CMD_map: cmdidx_T = 279;
const CMD_unmap: cmdidx_T = 505;
const EXPAND_NOTHING: c_int = 0;
const EXPAND_MAPPINGS: c_int = 16;

const kRetObject: c_int = 0;
const kNone: TriState = -1;
const kFalse: TriState = 0;
const kListLenUnknown: isize = -1;
const FC_LUAREF: c_int = 0x800;
const VAR_UNLOCKED: c_int = 0;

const KEYSET_OPTIDX_keymap__desc: u64 = 1;
const KEYSET_OPTIDX_keymap__callback: u64 = 8;

#[repr(C)]
pub struct mapblock_T {
    pub m_next: *mut mapblock_T,
    pub m_alt: *mut mapblock_T,
    pub m_keys: *mut c_char,
    pub m_str: *mut c_char,
    pub m_orig_str: *mut c_char,
    pub m_luaref: LuaRef,
    pub m_keylen: c_int,
    pub m_mode: c_int,
    pub m_simplified: c_int,
    pub m_noremap: c_int,
    pub m_silent: c_char,
    pub m_nowait: c_char,
    pub m_expr: c_char,
    pub m_script_ctx: sctx_T,
    pub m_desc: *mut c_char,
    pub m_replace_keycodes: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct typebuf_T {
    pub tb_buf: *mut u8,
    pub tb_noremap: *mut u8,
    pub tb_buflen: c_int,
    pub tb_off: c_int,
    pub tb_len: c_int,
    pub tb_maplen: c_int,
    pub tb_silent: c_int,
    pub tb_no_abbr_cnt: c_int,
    pub tb_change_cnt: c_int,
}

pub use crate::api::command::regmatch_T;

#[repr(C)]
#[derive(Clone, Copy)]
struct langmap_entry_T {
    from: c_int,
    to: c_int,
}

struct MapArguments {
    buffer: bool,
    expr: bool,
    noremap: bool,
    nowait: bool,
    script: bool,
    silent: bool,
    unique: bool,
    replace_keycodes: bool,
    lhs: [c_char; MAXMAPLEN + 1],
    lhs_len: usize,
    alt_lhs: [c_char; MAXMAPLEN + 1],
    alt_lhs_len: usize,
    rhs: *mut c_char,
    rhs_len: usize,
    rhs_lua: LuaRef,
    rhs_is_noop: bool,
    orig_rhs: *mut c_char,
    orig_rhs_len: usize,
    desc: *mut c_char,
}

impl MapArguments {
    const fn new() -> Self {
        Self {
            buffer: false,
            expr: false,
            noremap: false,
            nowait: false,
            script: false,
            silent: false,
            unique: false,
            replace_keycodes: false,
            lhs: [0; MAXMAPLEN + 1],
            lhs_len: 0,
            alt_lhs: [0; MAXMAPLEN + 1],
            alt_lhs_len: 0,
            rhs: ptr::null_mut(),
            rhs_len: 0,
            rhs_lua: LUA_NOREF,
            rhs_is_noop: false,
            orig_rhs: ptr::null_mut(),
            orig_rhs_len: 0,
            desc: ptr::null_mut(),
        }
    }
}

static mut first_abbr: *mut mapblock_T = ptr::null_mut();
static mut maphash: [*mut mapblock_T; MAX_MAPHASH] = [ptr::null_mut(); MAX_MAPHASH];
static mut map_luaid: c_int = 0;

static mut expand_mapmodes: c_int = 0;
static mut expand_isabbrev: bool = false;
static mut expand_buffer: bool = false;

static mut langmap_mapga: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: 0,
    ga_growsize: 0,
    ga_data: ptr::null_mut(),
};

static E_GLOBAL_ABBREVIATION_ALREADY_EXISTS_FOR_STR: &CStr =
    c"E224: Global abbreviation already exists for %s";
static E_GLOBAL_MAPPING_ALREADY_EXISTS_FOR_STR: &CStr =
    c"E225: Global mapping already exists for %s";
static E_ABBREVIATION_ALREADY_EXISTS_FOR_STR: &CStr = c"E226: Abbreviation already exists for %s";
static E_MAPPING_ALREADY_EXISTS_FOR_STR: &CStr = c"E227: Mapping already exists for %s";
static E_ENTRIES_MISSING_IN_MAPSET_DICT_ARGUMENT: &CStr =
    c"E460: Entries missing in mapset() dict argument";
static E_ILLEGAL_MAP_MODE_STRING_STR: &CStr = c"E1276: Illegal map mode string: '%s'";

unsafe extern "C" {

    fn atoi(s: *const c_char) -> c_int;
    fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char;
    fn strpbrk(s: *const c_char, accept: *const c_char) -> *mut c_char;
    fn fputc(c: c_int, stream: *mut FILE) -> c_int;
    fn fputs(s: *const c_char, stream: *mut FILE) -> c_int;

    fn gettext(msgid: *const c_char) -> *mut c_char;

}

#[inline]
const fn map_hash(mode: c_int, c1: u8) -> usize {
    if (mode & (MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING | MODE_TERMINAL)) != 0 {
        c1 as usize
    } else {
        (c1 ^ 0x80) as usize
    }
}

#[inline]
const fn is_special(c: c_int) -> bool {
    c < 0
}

#[inline]
const fn termcap2key(a: c_int, b: c_int) -> c_int {
    -(a + (b << 8))
}

#[inline]
const fn key2termcap0(x: c_int) -> c_int {
    (-x) & 0xff
}

#[inline]
const fn key2termcap1(x: c_int) -> c_int {
    (((-x) as u32) >> 8) as c_int & 0xff
}

#[inline]
const fn k_second(c: c_int) -> u8 {
    if c == K_SPECIAL as c_int {
        KS_SPECIAL
    } else if c == NUL as c_int {
        KS_ZERO
    } else {
        key2termcap0(c) as u8
    }
}

#[inline]
const fn k_third(c: c_int) -> u8 {
    if c == K_SPECIAL as c_int || c == NUL as c_int {
        KE_FILLER
    } else {
        key2termcap1(c) as u8
    }
}

#[inline]
const fn to_special(a: u8, b: u8) -> c_int {
    if a == KS_SPECIAL {
        K_SPECIAL as c_int
    } else if a == KS_ZERO {
        K_ZERO
    } else {
        termcap2key(a as c_int, b as c_int)
    }
}

#[inline]
const fn ascii_iswhite(c: c_int) -> bool {
    c == b' ' as c_int || c == b'\t' as c_int
}

#[inline]
const fn ascii_isspace(c: c_int) -> bool {
    (c >= 9 && c <= 13) || c == b' ' as c_int
}

#[inline]
const fn has_key(mask: u64, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

#[inline]
const fn cstr_literal(s: &'static CStr) -> NvimString {
    let bytes = s.to_bytes();
    NvimString {
        data: bytes.as_ptr() as *mut c_char,
        size: bytes.len(),
    }
}

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        let len = unsafe { CStr::from_ptr(s).to_bytes().len() };
        NvimString {
            data: s as *mut c_char,
            size: len,
        }
    }
}

#[inline]
const fn integer_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
    }
}

#[inline]
const fn string_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData { string: s },
    }
}

#[inline]
const fn luaref_obj(r: LuaRef) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeLuaRef,
        data: ObjectData { luaref: r },
    }
}

#[inline]
const fn dict_obj(d: Dict) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeDict,
        data: ObjectData { dict: d },
    }
}

#[inline]
unsafe fn cstr_as_obj(s: *const c_char) -> Object {
    unsafe { string_obj(cstr_as_string(s)) }
}

#[inline]
unsafe fn put_c(dict: &mut Dict, key: &'static CStr, value: Object) {
    unsafe {
        *dict.items.add(dict.size) = KeyValuePair {
            key: cstr_literal(key),
            value,
        };
        dict.size += 1;
    }
}

#[inline]
unsafe fn nlua_clear_ref(r: &mut LuaRef) {
    if *r != LUA_NOREF {
        unsafe { api_free_luaref(*r) };
        *r = LUA_NOREF;
    }
}

#[inline]
unsafe fn sourcing_lnum() -> linenr_T {
    unsafe {
        if !exestack.ga_data.is_null() && exestack.ga_len > 0 {
            (*(exestack.ga_data as *const estack_T).add((exestack.ga_len - 1) as usize)).es_lnum
        } else {
            0
        }
    }
}

/// Get the start of the hashed map list for "state" and first character "c".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_maphash_list(state: c_int, c: c_int) -> *mut mapblock_T {
    unsafe { maphash[map_hash(state, c as u8)] }
}

/// Get the buffer-local hashed map list for "state" and first character "c".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_buf_maphash_list(state: c_int, c: c_int) -> *mut mapblock_T {
    unsafe { (*curbuf).b_maphash[map_hash(state, c as u8)].cast::<mapblock_T>() }
}

/// Finds the Lua mapping whose `m_str` embeds `id`.
unsafe fn map_luaid_scan(
    mappings: *const *mut mapblock_T,
    size: usize,
    id: c_int,
) -> *mut mapblock_T {
    unsafe {
        for i in 0..size {
            let mut mp = *mappings.add(i);
            while !mp.is_null() {
                if (*mp).m_luaref != LUA_NOREF && atoi((*mp).m_str.add(3)) == id {
                    return mp;
                }
                mp = (*mp).m_next;
            }
        }
        ptr::null_mut()
    }
}

/// Resolves a Lua-mapping id to its callback.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn map_luaid_get(n: c_int) -> LuaRef {
    unsafe {
        let mut mp = map_luaid_scan(
            (&raw const maphash).cast::<*mut mapblock_T>(),
            MAX_MAPHASH,
            n,
        );
        if mp.is_null() {
            mp = map_luaid_scan(&raw const first_abbr, 1, n);
        }
        let mut fb = firstbuf;
        while !fb.is_null() {
            if mp.is_null() {
                mp = map_luaid_scan(
                    (&raw const (*fb).b_maphash).cast::<*mut mapblock_T>(),
                    MAX_MAPHASH,
                    n,
                );
            }
            if mp.is_null() {
                mp = map_luaid_scan(
                    (&raw const (*fb).b_first_abbr).cast::<*mut mapblock_T>(),
                    1,
                    n,
                );
            }
            fb = (*fb).b_next;
        }
        if !mp.is_null() {
            (*mp).m_luaref
        } else {
            LUA_NOREF
        }
    }
}

/// Delete one entry from the abbrlist or maphash[].
/// "mpp" is a pointer to the m_next field of the PREVIOUS entry!
unsafe fn mapblock_free(mpp: *mut *mut mapblock_T) {
    unsafe {
        let mp = *mpp;
        xfree((*mp).m_keys.cast::<c_void>());
        if !(*mp).m_alt.is_null() {
            (*(*mp).m_alt).m_alt = ptr::null_mut();
        } else {
            nlua_clear_ref(&mut (*mp).m_luaref);
            xfree((*mp).m_str.cast::<c_void>());
            xfree((*mp).m_orig_str.cast::<c_void>());
            xfree((*mp).m_desc.cast::<c_void>());
        }
        *mpp = (*mp).m_next;
        xfree(mp.cast::<c_void>());
    }
}

/// Put characters to represent the map mode in a string buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn map_mode_to_chars(mode: c_int, buf: *mut c_char) {
    unsafe {
        let mut p = buf;
        if (mode & (MODE_INSERT | MODE_CMDLINE)) == (MODE_INSERT | MODE_CMDLINE) {
            *p = b'!' as c_char;
            p = p.add(1);
        } else if (mode & MODE_INSERT) != 0 {
            *p = b'i' as c_char;
            p = p.add(1);
        } else if (mode & MODE_LANGMAP) != 0 {
            *p = b'l' as c_char;
            p = p.add(1);
        } else if (mode & MODE_CMDLINE) != 0 {
            *p = b'c' as c_char;
            p = p.add(1);
        } else if (mode & (MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING))
            == (MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING)
        {
            *p = b' ' as c_char;
            p = p.add(1);
        } else {
            if (mode & MODE_NORMAL) != 0 {
                *p = b'n' as c_char;
                p = p.add(1);
            }
            if (mode & MODE_OP_PENDING) != 0 {
                *p = b'o' as c_char;
                p = p.add(1);
            }
            if (mode & MODE_TERMINAL) != 0 {
                *p = b't' as c_char;
                p = p.add(1);
            }
            if (mode & (MODE_VISUAL | MODE_SELECT)) == (MODE_VISUAL | MODE_SELECT) {
                *p = b'v' as c_char;
                p = p.add(1);
            } else {
                if (mode & MODE_VISUAL) != 0 {
                    *p = b'x' as c_char;
                    p = p.add(1);
                }
                if (mode & MODE_SELECT) != 0 {
                    *p = b's' as c_char;
                    p = p.add(1);
                }
            }
        }
        *p = NUL;
    }
}

unsafe fn showmap(mp: *mut mapblock_T, local: bool) {
    unsafe {
        if message_filtered((*mp).m_keys)
            && message_filtered((*mp).m_str)
            && ((*mp).m_desc.is_null() || message_filtered((*mp).m_desc))
        {
            return;
        }

        if msg_col > 0 || msg_silent != 0 {
            msg_putchar(b'\n' as c_int);
            if got_int {
                return;
            }
        }

        let mut mapchars = [0 as c_char; 7];
        map_mode_to_chars((*mp).m_mode, mapchars.as_mut_ptr());
        msg_puts(mapchars.as_ptr());
        let mut len = CStr::from_ptr(mapchars.as_ptr()).to_bytes().len();

        loop {
            len += 1;
            if len > 3 {
                break;
            }
            msg_putchar(b' ' as c_int);
        }

        len = msg_outtrans_special((*mp).m_keys, true, 0) as usize;
        loop {
            msg_putchar(b' ' as c_int);
            len += 1;
            if len >= 12 {
                break;
            }
        }

        if (*mp).m_noremap == REMAP_NONE {
            msg_puts_hl(c"*".as_ptr(), HLF_8, false);
        } else if (*mp).m_noremap == REMAP_SCRIPT {
            msg_puts_hl(c"&".as_ptr(), HLF_8, false);
        } else {
            msg_putchar(b' ' as c_int);
        }

        if local {
            msg_putchar(b'@' as c_int);
        } else {
            msg_putchar(b' ' as c_int);
        }

        if (*mp).m_luaref != LUA_NOREF {
            let str_ptr = nlua_funcref_str((*mp).m_luaref, ptr::null_mut(), true);
            msg_puts_hl(str_ptr, HLF_8, false);
            xfree(str_ptr.cast::<c_void>());
        } else if *(*mp).m_str == NUL {
            msg_puts_hl(c"<Nop>".as_ptr(), HLF_8, false);
        } else {
            msg_outtrans_special((*mp).m_str, false, 0);
        }

        if !(*mp).m_desc.is_null() {
            msg_puts(c"\n                 ".as_ptr());
            msg_puts((*mp).m_desc);
        }
        if p_verbose > 0 {
            last_set_msg((*mp).m_script_ctx);
        }
        msg_clr_eos();
    }
}

unsafe fn set_maparg_lhs_rhs(
    orig_lhs: *const c_char,
    orig_lhs_len: usize,
    orig_rhs: *const c_char,
    orig_rhs_len: usize,
    rhs_lua: LuaRef,
    cpo_val: *const c_char,
    mapargs: &mut MapArguments,
) -> bool {
    unsafe {
        mapargs.rhs_lua = rhs_lua;
        let mut lhs_buf = [0 as c_char; 128];

        let mut did_simplify = false;
        let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
        let mut bufarg = lhs_buf.as_mut_ptr();
        let mut replaced = replace_termcodes(
            orig_lhs,
            orig_lhs_len,
            &mut bufarg,
            0,
            flags,
            &mut did_simplify,
            cpo_val,
        );
        if replaced.is_null() {
            return false;
        }
        mapargs.lhs_len = CStr::from_ptr(replaced).to_bytes().len();
        xstrlcpy(mapargs.lhs.as_mut_ptr(), replaced, mapargs.lhs.len());
        if did_simplify {
            replaced = replace_termcodes(
                orig_lhs,
                orig_lhs_len,
                &mut bufarg,
                0,
                flags | REPTERM_NO_SIMPLIFY,
                ptr::null_mut(),
                cpo_val,
            );
            if replaced.is_null() {
                return false;
            }
            mapargs.alt_lhs_len = CStr::from_ptr(replaced).to_bytes().len();
            xstrlcpy(
                mapargs.alt_lhs.as_mut_ptr(),
                replaced,
                mapargs.alt_lhs.len(),
            );
        } else {
            mapargs.alt_lhs_len = 0;
        }

        set_maparg_rhs(orig_rhs, orig_rhs_len, rhs_lua, 0, cpo_val, mapargs);
        true
    }
}

unsafe fn set_maparg_rhs(
    orig_rhs: *const c_char,
    orig_rhs_len: usize,
    rhs_lua: LuaRef,
    sid: scid_T,
    cpo_val: *const c_char,
    mapargs: &mut MapArguments,
) {
    unsafe {
        mapargs.rhs_lua = rhs_lua;

        if rhs_lua == LUA_NOREF {
            mapargs.orig_rhs_len = orig_rhs_len;
            mapargs.orig_rhs = xcalloc(mapargs.orig_rhs_len + 1, 1).cast::<c_char>();
            xmemcpyz(
                mapargs.orig_rhs.cast::<c_void>(),
                orig_rhs.cast::<c_void>(),
                mapargs.orig_rhs_len,
            );
            if CStr::from_ptr(orig_rhs)
                .to_bytes()
                .eq_ignore_ascii_case(b"<nop>")
            {
                mapargs.rhs = xcalloc(1, 1).cast::<c_char>();
                mapargs.rhs_len = 0;
                mapargs.rhs_is_noop = true;
            } else {
                let mut rhs_buf: *mut c_char = ptr::null_mut();
                let replaced = replace_termcodes(
                    orig_rhs,
                    orig_rhs_len,
                    &mut rhs_buf,
                    sid,
                    REPTERM_DO_LT,
                    ptr::null_mut(),
                    cpo_val,
                );
                mapargs.rhs_len = CStr::from_ptr(replaced).to_bytes().len();
                mapargs.rhs_is_noop = orig_rhs_len != 0 && mapargs.rhs_len == 0;
                mapargs.rhs = replaced;
            }
        } else {
            let mut tmp_buf = [0 as c_char; 64];
            mapargs.orig_rhs = xcalloc(1, 1).cast::<c_char>();
            mapargs.orig_rhs_len = 0;
            map_luaid += 1;
            mapargs.rhs_len = vim_snprintf!(
                tmp_buf.as_mut_ptr(),
                tmp_buf.len(),
                c"%c%c%c%d\r".as_ptr(),
                K_SPECIAL as c_int,
                KS_EXTRA as c_int,
                KE_LUA as c_int,
                map_luaid,
            ) as usize;
            mapargs.rhs = xstrdup(tmp_buf.as_ptr());
        }
    }
}

unsafe fn str_to_mapargs(
    strargs: *const c_char,
    is_unmap: bool,
    mapargs: &mut MapArguments,
) -> c_int {
    unsafe {
        let mut to_parse = skipwhite(strargs);
        *mapargs = MapArguments::new();
        mapargs.rhs_lua = 0;

        loop {
            let bytes = CStr::from_ptr(to_parse).to_bytes();
            if bytes.starts_with(b"<buffer>") {
                to_parse = skipwhite(to_parse.add(8));
                mapargs.buffer = true;
                continue;
            }
            if bytes.starts_with(b"<nowait>") {
                to_parse = skipwhite(to_parse.add(8));
                mapargs.nowait = true;
                continue;
            }
            if bytes.starts_with(b"<silent>") {
                to_parse = skipwhite(to_parse.add(8));
                mapargs.silent = true;
                continue;
            }
            if bytes.starts_with(b"<special>") {
                to_parse = skipwhite(to_parse.add(9));
                continue;
            }
            if bytes.starts_with(b"<script>") {
                to_parse = skipwhite(to_parse.add(8));
                mapargs.script = true;
                continue;
            }
            if bytes.starts_with(b"<expr>") {
                to_parse = skipwhite(to_parse.add(6));
                mapargs.expr = true;
                continue;
            }
            if bytes.starts_with(b"<unique>") {
                to_parse = skipwhite(to_parse.add(8));
                mapargs.unique = true;
                continue;
            }
            break;
        }

        let mut lhs_end = to_parse;
        let do_backslash = vim_strchr(p_cpo, kCpoBslash).is_null();
        while *lhs_end != NUL && (is_unmap || !ascii_iswhite(*lhs_end as u8 as c_int)) {
            if (*lhs_end == Ctrl_V as c_char || (do_backslash && *lhs_end == b'\\' as c_char))
                && *lhs_end.add(1) != NUL
            {
                lhs_end = lhs_end.add(1);
            }
            lhs_end = lhs_end.add(1);
        }

        let rhs_start = skipwhite(lhs_end);

        let orig_lhs_len = (lhs_end as usize) - (to_parse as usize);
        if orig_lhs_len >= 256 {
            return 1;
        }
        let mut lhs_to_replace = [0 as c_char; 256];
        xmemcpyz(
            lhs_to_replace.as_mut_ptr().cast::<c_void>(),
            to_parse.cast::<c_void>(),
            orig_lhs_len,
        );

        let orig_rhs_len = CStr::from_ptr(rhs_start).to_bytes().len();
        if !set_maparg_lhs_rhs(
            lhs_to_replace.as_ptr(),
            orig_lhs_len,
            rhs_start,
            orig_rhs_len,
            LUA_NOREF,
            p_cpo,
            mapargs,
        ) {
            return 1;
        }

        if mapargs.lhs_len > MAXMAPLEN {
            return 1;
        }
        0
    }
}

unsafe fn map_add(
    buf: *mut buf_T,
    map_table: *mut *mut mapblock_T,
    abbr_table: *mut *mut mapblock_T,
    keys: *const c_char,
    args: &MapArguments,
    noremap: c_int,
    mode: c_int,
    is_abbr: bool,
    sid: scid_T,
    lnum: linenr_T,
    simplified: bool,
) -> *mut mapblock_T {
    unsafe {
        let mp = xcalloc(1, std::mem::size_of::<mapblock_T>()).cast::<mapblock_T>();

        if *keys == Ctrl_C {
            if ptr::eq(
                map_table,
                (&raw mut (*buf).b_maphash).cast::<*mut mapblock_T>(),
            ) {
                (*buf).b_mapped_ctrl_c |= mode;
            } else {
                mapped_ctrl_c |= mode;
            }
        }

        (*mp).m_keys = xstrdup(keys);
        (*mp).m_str = args.rhs;
        (*mp).m_orig_str = args.orig_rhs;
        (*mp).m_luaref = args.rhs_lua;
        (*mp).m_keylen = CStr::from_ptr((*mp).m_keys).to_bytes().len() as c_int;
        (*mp).m_noremap = noremap;
        (*mp).m_nowait = args.nowait as c_char;
        (*mp).m_silent = args.silent as c_char;
        (*mp).m_mode = mode;
        (*mp).m_simplified = c_int::from(simplified);
        (*mp).m_expr = args.expr as c_char;
        (*mp).m_replace_keycodes = args.replace_keycodes;
        if sid != 0 {
            (*mp).m_script_ctx.sc_sid = sid;
            (*mp).m_script_ctx.sc_lnum = lnum;
        } else {
            (*mp).m_script_ctx = current_sctx;
            (*mp).m_script_ctx.sc_lnum += sourcing_lnum();
            nlua_set_sctx(&mut (*mp).m_script_ctx);
        }
        (*mp).m_desc = args.desc;

        if is_abbr {
            (*mp).m_next = *abbr_table;
            *abbr_table = mp;
        } else {
            let n = map_hash((*mp).m_mode, *(*mp).m_keys as u8);
            (*mp).m_next = *map_table.add(n);
            *map_table.add(n) = mp;
        }
        mp
    }
}

unsafe fn buf_do_map(
    mut maptype: c_int,
    args: &mut MapArguments,
    mode: c_int,
    is_abbrev: bool,
    buf: *mut buf_T,
) -> c_int {
    unsafe {
        let mut retval: c_int = 0;

        let buf_maphash = (&raw mut (*buf).b_maphash).cast::<*mut mapblock_T>();
        let global_maphash = (&raw mut maphash).cast::<*mut mapblock_T>();
        let map_table = if args.buffer {
            buf_maphash
        } else {
            global_maphash
        };
        let abbr_table = if args.buffer {
            (&raw mut (*buf).b_first_abbr).cast::<*mut mapblock_T>()
        } else {
            &raw mut first_abbr
        };
        let mut mp_result: [*mut mapblock_T; 2] = [ptr::null_mut(), ptr::null_mut()];

        let mut unmap_lhs_only = false;
        if maptype == MAPTYPE_UNMAP_LHS {
            unmap_lhs_only = true;
            maptype = MAPTYPE_UNMAP;
        }

        let noremap = if args.script {
            REMAP_SCRIPT
        } else if maptype == MAPTYPE_NOREMAP {
            REMAP_NONE
        } else {
            REMAP_YES
        };

        let has_lhs = args.lhs[0] != NUL;
        let has_rhs = args.rhs_lua != LUA_NOREF
            || (!args.rhs.is_null() && *args.rhs != NUL)
            || args.rhs_is_noop;
        let do_print = !has_lhs || (maptype != MAPTYPE_UNMAP && !has_rhs);
        if do_print {
            msg_ext_set_kind(c"list_cmd".as_ptr());
        }

        if maptype == MAPTYPE_UNMAP && !has_lhs {
            return 1;
        }

        let mut lhs = args.lhs.as_ptr();
        let did_simplify = args.alt_lhs_len != 0;

        'outer: {
            for keyround in 1..=2 {
                let mut did_it = false;
                let mut did_local = false;
                let keyround1_simplified = keyround == 1 && did_simplify;
                let mut len = args.lhs_len as c_int;

                if keyround == 2 {
                    if !did_simplify {
                        break;
                    }
                    lhs = args.alt_lhs.as_ptr();
                    len = args.alt_lhs_len as c_int;
                } else if did_simplify && do_print {
                    lhs = args.alt_lhs.as_ptr();
                    len = args.alt_lhs_len as c_int;
                }

                if has_lhs {
                    if (len as usize) > MAXMAPLEN {
                        retval = 1;
                        break 'outer;
                    }

                    if is_abbrev && maptype != MAPTYPE_UNMAP {
                        let mut same: c_int = -1;
                        let mut keys_unescaped = [0 as c_char; MAXMAPLEN + 1];
                        xmemcpyz(
                            keys_unescaped.as_mut_ptr().cast::<c_void>(),
                            lhs.cast::<c_void>(),
                            len as usize,
                        );
                        let keys_unescaped_len = vim_unescape_ks(keys_unescaped.as_mut_ptr());
                        let mut p = keys_unescaped.as_ptr();
                        let end_p = p.add(keys_unescaped_len);

                        let first = vim_iswordp(p);
                        let mut last = first;
                        p = p.add(utfc_ptr2len(p) as usize);
                        let mut n: c_int = 1;
                        while (p as usize) < (end_p as usize) {
                            n += 1;
                            last = vim_iswordp(p);
                            if same == -1 && last != first {
                                same = n - 1;
                            }
                            p = p.add(utfc_ptr2len(p) as usize);
                        }
                        if last && n > 2 && same >= 0 && same < n - 1 {
                            retval = 1;
                            break 'outer;
                        }

                        for i in 0..(len as usize) {
                            if ascii_iswhite(*lhs.add(i) as u8 as c_int) {
                                retval = 1;
                                break 'outer;
                            }
                        }
                    }
                }

                if has_lhs && has_rhs && is_abbrev {
                    no_abbr = false;
                }

                if do_print {
                    msg_start();
                }

                if args.unique
                    && ptr::eq(map_table, buf_maphash)
                    && has_lhs
                    && has_rhs
                    && maptype != MAPTYPE_UNMAP
                {
                    let lhs_slice = std::slice::from_raw_parts(lhs.cast::<u8>(), len as usize);
                    for hash in 0..256 {
                        if got_int {
                            break;
                        }
                        let mut mp = if is_abbrev {
                            if hash != 0 {
                                break;
                            }
                            first_abbr
                        } else {
                            maphash[hash]
                        };
                        while !mp.is_null() && !got_int {
                            if ((*mp).m_mode & mode) != 0 && (*mp).m_keylen == len {
                                let mp_slice = std::slice::from_raw_parts(
                                    (*mp).m_keys.cast::<u8>(),
                                    len as usize,
                                );
                                if mp_slice == lhs_slice {
                                    retval = 6;
                                    break 'outer;
                                }
                            }
                            mp = (*mp).m_next;
                        }
                    }
                }

                if !ptr::eq(map_table, buf_maphash) && !has_rhs && maptype != MAPTYPE_UNMAP {
                    for hash in 0..256 {
                        if got_int {
                            break;
                        }
                        let mut mp = if is_abbrev {
                            if hash != 0 {
                                break;
                            }
                            (*buf).b_first_abbr.cast::<mapblock_T>()
                        } else {
                            (*buf).b_maphash[hash].cast::<mapblock_T>()
                        };
                        while !mp.is_null() && !got_int {
                            if (*mp).m_simplified == 0 && ((*mp).m_mode & mode) != 0 {
                                if !has_lhs {
                                    showmap(mp, true);
                                    did_local = true;
                                } else {
                                    let n = (*mp).m_keylen;
                                    let cmp_len = n.min(len) as usize;
                                    let mp_slice = std::slice::from_raw_parts(
                                        (*mp).m_keys.cast::<u8>(),
                                        cmp_len,
                                    );
                                    let lhs_slice =
                                        std::slice::from_raw_parts(lhs.cast::<u8>(), cmp_len);
                                    if mp_slice == lhs_slice {
                                        showmap(mp, true);
                                        did_local = true;
                                    }
                                }
                            }
                            mp = (*mp).m_next;
                        }
                    }
                }

                let num_rounds = if maptype == MAPTYPE_UNMAP && !unmap_lhs_only {
                    2
                } else {
                    1
                };
                for round in 0..num_rounds {
                    if did_it || got_int {
                        break;
                    }
                    let (hash_start, hash_end) = if (round == 0 && has_lhs) || is_abbrev {
                        let start = if is_abbrev {
                            0
                        } else {
                            map_hash(mode, *lhs as u8)
                        };
                        (start, start + 1)
                    } else {
                        (0, 256)
                    };
                    for hash in hash_start..hash_end {
                        if got_int {
                            break;
                        }
                        let mut mpp = if is_abbrev {
                            abbr_table
                        } else {
                            map_table.add(hash)
                        };
                        while !(*mpp).is_null() && !got_int {
                            let mp = *mpp;
                            if ((*mp).m_mode & mode) == 0 {
                                mpp = &raw mut (*mp).m_next;
                                continue;
                            }
                            if !has_lhs {
                                if (*mp).m_simplified == 0 {
                                    showmap(mp, !ptr::eq(map_table, global_maphash));
                                    did_it = true;
                                }
                            } else {
                                let (n, p) = if round != 0 {
                                    (
                                        CStr::from_ptr((*mp).m_str).to_bytes().len() as c_int,
                                        (*mp).m_str as *const c_char,
                                    )
                                } else {
                                    ((*mp).m_keylen, (*mp).m_keys as *const c_char)
                                };
                                let cmp_len = n.min(len) as usize;
                                let p_slice = std::slice::from_raw_parts(p.cast::<u8>(), cmp_len);
                                let lhs_slice =
                                    std::slice::from_raw_parts(lhs.cast::<u8>(), cmp_len);
                                if p_slice == lhs_slice {
                                    if maptype == MAPTYPE_UNMAP {
                                        if n != len
                                            && (!is_abbrev
                                                || round != 0
                                                || n > len
                                                || *skipwhite(lhs.add(n as usize)) != NUL)
                                        {
                                            mpp = &raw mut (*mp).m_next;
                                            continue;
                                        }
                                        if keyround1_simplified && (*mp).m_simplified == 0 {
                                            break;
                                        }
                                        (*mp).m_mode &= !mode;
                                        did_it = true;
                                    } else if !has_rhs {
                                        if (*mp).m_simplified == 0 {
                                            showmap(mp, !ptr::eq(map_table, global_maphash));
                                            did_it = true;
                                        }
                                    } else if n != len {
                                        mpp = &raw mut (*mp).m_next;
                                        continue;
                                    } else if keyround1_simplified && (*mp).m_simplified == 0 {
                                        did_it = true;
                                        break;
                                    } else if args.unique {
                                        retval = 5;
                                        break 'outer;
                                    } else {
                                        (*mp).m_mode &= !mode;
                                        if (*mp).m_mode == 0 && !did_it {
                                            if !(*mp).m_alt.is_null() {
                                                (*(*mp).m_alt).m_alt = ptr::null_mut();
                                                (*mp).m_alt = ptr::null_mut();
                                            } else {
                                                nlua_clear_ref(&mut (*mp).m_luaref);
                                                xfree((*mp).m_str.cast::<c_void>());
                                                xfree((*mp).m_orig_str.cast::<c_void>());
                                                xfree((*mp).m_desc.cast::<c_void>());
                                            }
                                            (*mp).m_str = args.rhs;
                                            (*mp).m_orig_str = args.orig_rhs;
                                            (*mp).m_luaref = args.rhs_lua;
                                            (*mp).m_noremap = noremap;
                                            (*mp).m_nowait = args.nowait as c_char;
                                            (*mp).m_silent = args.silent as c_char;
                                            (*mp).m_mode = mode;
                                            (*mp).m_simplified = c_int::from(keyround1_simplified);
                                            (*mp).m_expr = args.expr as c_char;
                                            (*mp).m_replace_keycodes = args.replace_keycodes;
                                            (*mp).m_script_ctx = current_sctx;
                                            (*mp).m_script_ctx.sc_lnum += sourcing_lnum();
                                            nlua_set_sctx(&mut (*mp).m_script_ctx);
                                            (*mp).m_desc = args.desc;
                                            mp_result[keyround - 1] = mp;
                                            did_it = true;
                                        }
                                    }
                                    if (*mp).m_mode == 0 {
                                        mapblock_free(mpp);
                                        continue;
                                    }

                                    let new_hash = map_hash((*mp).m_mode, *(*mp).m_keys as u8);
                                    if !is_abbrev && new_hash != hash {
                                        *mpp = (*mp).m_next;
                                        (*mp).m_next = *map_table.add(new_hash);
                                        *map_table.add(new_hash) = mp;
                                        continue;
                                    }
                                }
                            }
                            mpp = &raw mut (*mp).m_next;
                        }
                    }
                }

                if maptype == MAPTYPE_UNMAP {
                    if !did_it {
                        if !keyround1_simplified {
                            retval = 2;
                        }
                    } else if *lhs == Ctrl_C {
                        if ptr::eq(map_table, buf_maphash) {
                            (*buf).b_mapped_ctrl_c &= !mode;
                        } else {
                            mapped_ctrl_c &= !mode;
                        }
                    }
                    continue;
                }

                if !has_lhs || !has_rhs {
                    if !did_it && !did_local {
                        if is_abbrev {
                            msg(gettext(c"No abbreviation found".as_ptr()), 0);
                        } else {
                            msg(gettext(c"No mapping found".as_ptr()), 0);
                        }
                    }
                    break 'outer;
                }

                if did_it {
                    continue;
                }

                mp_result[keyround - 1] = map_add(
                    buf,
                    map_table,
                    abbr_table,
                    lhs,
                    args,
                    noremap,
                    mode,
                    is_abbrev,
                    0,
                    0,
                    keyround1_simplified,
                );
            }

            if !mp_result[0].is_null() && !mp_result[1].is_null() {
                (*mp_result[0]).m_alt = mp_result[1];
                (*mp_result[1]).m_alt = mp_result[0];
            }
        }

        if !mp_result[0].is_null() || !mp_result[1].is_null() {
            args.rhs = ptr::null_mut();
            args.orig_rhs = ptr::null_mut();
            args.rhs_lua = LUA_NOREF;
            args.desc = ptr::null_mut();
        }
        retval
    }
}

/// Set or remove a mapping or an abbreviation in the current buffer, OR
/// display (matching) mappings/abbreviations.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_map(
    maptype: c_int,
    arg: *mut c_char,
    mode: c_int,
    is_abbrev: bool,
) -> c_int {
    unsafe {
        let mut parsed_args = MapArguments::new();
        let mut result = str_to_mapargs(arg, maptype == MAPTYPE_UNMAP, &mut parsed_args);
        match result {
            0 => {
                result = buf_do_map(maptype, &mut parsed_args, mode, is_abbrev, curbuf);
            }
            1 => {}
            _ => {
                debug_assert!(false, "Unknown return code from str_to_mapargs!");
                result = -1;
            }
        }

        xfree(parsed_args.rhs.cast::<c_void>());
        xfree(parsed_args.orig_rhs.cast::<c_void>());
        result
    }
}

/// Get the mapping mode from the command name.
unsafe fn get_map_mode(cmdp: *mut *mut c_char, forceit: bool) -> c_int {
    unsafe {
        let mut p = *cmdp;
        let modec = *p as u8;
        p = p.add(1);
        let mode = if modec == b'i' {
            MODE_INSERT
        } else if modec == b'l' {
            MODE_LANGMAP
        } else if modec == b'c' {
            MODE_CMDLINE
        } else if modec == b'n' && *p != b'o' as c_char {
            MODE_NORMAL
        } else if modec == b'v' {
            MODE_VISUAL | MODE_SELECT
        } else if modec == b'x' {
            MODE_VISUAL
        } else if modec == b's' {
            MODE_SELECT
        } else if modec == b'o' {
            MODE_OP_PENDING
        } else if modec == b't' {
            MODE_TERMINAL
        } else {
            p = p.sub(1);
            if forceit {
                MODE_INSERT | MODE_CMDLINE
            } else {
                MODE_VISUAL | MODE_SELECT | MODE_NORMAL | MODE_OP_PENDING
            }
        };

        *cmdp = p;
        mode
    }
}

unsafe fn do_mapclear(mut cmdp: *mut c_char, arg: *mut c_char, forceit: c_int, abbr: c_int) {
    unsafe {
        let local = CStr::from_ptr(arg).to_bytes() == b"<buffer>";
        if !local && *arg != NUL {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mode = get_map_mode(&mut cmdp, forceit != 0);
        map_clear_mode(curbuf, mode, local, abbr != 0);
    }
}

/// Clear all mappings in "mode".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn map_clear_mode(buf: *mut buf_T, mode: c_int, local: bool, abbr: bool) {
    unsafe {
        let buf_maphash = (&raw mut (*buf).b_maphash).cast::<*mut mapblock_T>();
        let global_maphash = (&raw mut maphash).cast::<*mut mapblock_T>();
        for hash in 0..256 {
            let mut mpp = if abbr {
                if hash > 0 {
                    break;
                }
                if local {
                    (&raw mut (*buf).b_first_abbr).cast::<*mut mapblock_T>()
                } else {
                    &raw mut first_abbr
                }
            } else if local {
                buf_maphash.add(hash)
            } else {
                global_maphash.add(hash)
            };
            while !(*mpp).is_null() {
                let mp = *mpp;
                if ((*mp).m_mode & mode) != 0 {
                    (*mp).m_mode &= !mode;
                    if (*mp).m_mode == 0 {
                        mapblock_free(mpp);
                        continue;
                    }
                    let new_hash = map_hash((*mp).m_mode, *(*mp).m_keys as u8);
                    if !abbr && new_hash != hash {
                        *mpp = (*mp).m_next;
                        if local {
                            (*mp).m_next = *buf_maphash.add(new_hash);
                            *buf_maphash.add(new_hash) = mp;
                        } else {
                            (*mp).m_next = *global_maphash.add(new_hash);
                            *global_maphash.add(new_hash) = mp;
                        }
                        continue;
                    }
                }
                mpp = &raw mut (*mp).m_next;
            }
        }
    }
}

/// Check if a map exists that has given string in the rhs.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn map_to_exists(
    str_ptr: *const c_char,
    modechars: *const c_char,
    abbr: bool,
) -> bool {
    unsafe {
        let mut mode: c_int = 0;
        let mut buf: *mut c_char = ptr::null_mut();
        let rhs = replace_termcodes(
            str_ptr,
            CStr::from_ptr(str_ptr).to_bytes().len(),
            &mut buf,
            0,
            REPTERM_DO_LT,
            ptr::null_mut(),
            p_cpo,
        );

        let mode_bytes = CStr::from_ptr(modechars).to_bytes();
        if mode_bytes.contains(&b'n') {
            mode |= MODE_NORMAL;
        }
        if mode_bytes.contains(&b'v') {
            mode |= MODE_VISUAL | MODE_SELECT;
        }
        if mode_bytes.contains(&b'x') {
            mode |= MODE_VISUAL;
        }
        if mode_bytes.contains(&b's') {
            mode |= MODE_SELECT;
        }
        if mode_bytes.contains(&b'o') {
            mode |= MODE_OP_PENDING;
        }
        if mode_bytes.contains(&b'i') {
            mode |= MODE_INSERT;
        }
        if mode_bytes.contains(&b'l') {
            mode |= MODE_LANGMAP;
        }
        if mode_bytes.contains(&b'c') {
            mode |= MODE_CMDLINE;
        }

        let retval = map_to_exists_mode(rhs, mode, abbr);
        xfree(buf.cast::<c_void>());
        retval
    }
}

/// Check if a map exists that has given string in the rhs.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn map_to_exists_mode(rhs: *const c_char, mode: c_int, abbr: bool) -> bool {
    unsafe {
        let mut exp_buffer = false;

        loop {
            for hash in 0..256 {
                let mut mp = if abbr {
                    if hash > 0 {
                        break;
                    }
                    if exp_buffer {
                        (*curbuf).b_first_abbr.cast::<mapblock_T>()
                    } else {
                        first_abbr
                    }
                } else if exp_buffer {
                    (*curbuf).b_maphash[hash].cast::<mapblock_T>()
                } else {
                    maphash[hash]
                };
                while !mp.is_null() {
                    if ((*mp).m_mode & mode) != 0 && !strstr((*mp).m_str, rhs).is_null() {
                        return true;
                    }
                    mp = (*mp).m_next;
                }
            }
            if exp_buffer {
                break;
            }
            exp_buffer = true;
        }

        false
    }
}

unsafe fn translate_mapping(str_in: *const c_char, cpo_val: *const c_char) -> *mut c_char {
    unsafe {
        let mut str_ptr = str_in.cast::<u8>();
        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&mut ga, 1, 40);

        let cpo_bslash = !vim_strchr(cpo_val, kCpoBslash).is_null();

        while *str_ptr != 0 {
            let mut c = *str_ptr as c_int;
            if c == K_SPECIAL as c_int && *str_ptr.add(1) != 0 && *str_ptr.add(2) != 0 {
                let mut modifiers: c_int = 0;
                if *str_ptr.add(1) == KS_MODIFIER {
                    str_ptr = str_ptr.add(1);
                    str_ptr = str_ptr.add(1);
                    modifiers = *str_ptr as c_int;
                    str_ptr = str_ptr.add(1);
                    c = *str_ptr as c_int;
                }

                if c == K_SPECIAL as c_int && *str_ptr.add(1) != 0 && *str_ptr.add(2) != 0 {
                    c = to_special(*str_ptr.add(1), *str_ptr.add(2));
                    if c == K_ZERO {
                        c = NUL as c_int;
                    }
                    str_ptr = str_ptr.add(2);
                }
                if is_special(c) || modifiers != 0 {
                    ga_concat(&mut ga, get_special_key_name(c, modifiers));
                    str_ptr = str_ptr.add(1);
                    continue;
                }
            }

            if c == b' ' as c_int
                || c == b'\t' as c_int
                || c == Ctrl_J
                || c == Ctrl_V
                || c == b'<' as c_int
                || (c == b'\\' as c_int && !cpo_bslash)
            {
                ga_append(&mut ga, if cpo_bslash { Ctrl_V as u8 } else { b'\\' });
            }

            if c != 0 {
                ga_append(&mut ga, c as u8);
            }
            str_ptr = str_ptr.add(1);
        }
        ga_append(&mut ga, 0);
        ga.ga_data.cast::<c_char>()
    }
}

/// Work out what to complete when doing command line completion of mapping
/// or abbreviation names.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_context_in_map_cmd(
    xp: *mut expand_T,
    mut cmd: *mut c_char,
    mut arg: *mut c_char,
    forceit: bool,
    isabbrev: bool,
    isunmap: bool,
    cmdidx: crate::cmdexpand::cmdidx_T,
) -> *mut c_char {
    unsafe {
        if forceit
            && cmdidx != crate::cmdexpand::cmdidx_T::CMD_map
            && cmdidx != crate::cmdexpand::cmdidx_T::CMD_unmap
        {
            (*xp).xp_context = EXPAND_NOTHING;
        } else {
            if isunmap {
                expand_mapmodes = get_map_mode(&mut cmd, forceit || isabbrev);
            } else {
                expand_mapmodes = MODE_INSERT | MODE_CMDLINE;
                if !isabbrev {
                    expand_mapmodes |= MODE_VISUAL | MODE_SELECT | MODE_NORMAL | MODE_OP_PENDING;
                }
            }
            expand_isabbrev = isabbrev;
            (*xp).xp_context = EXPAND_MAPPINGS;
            expand_buffer = false;
            loop {
                let bytes = CStr::from_ptr(arg).to_bytes();
                if bytes.starts_with(b"<buffer>") {
                    expand_buffer = true;
                    arg = skipwhite(arg.add(8));
                    continue;
                }
                if bytes.starts_with(b"<unique>") {
                    arg = skipwhite(arg.add(8));
                    continue;
                }
                if bytes.starts_with(b"<nowait>") {
                    arg = skipwhite(arg.add(8));
                    continue;
                }
                if bytes.starts_with(b"<silent>") {
                    arg = skipwhite(arg.add(8));
                    continue;
                }
                if bytes.starts_with(b"<special>") {
                    arg = skipwhite(arg.add(9));
                    continue;
                }
                if bytes.starts_with(b"<script>") {
                    arg = skipwhite(arg.add(8));
                    continue;
                }
                if bytes.starts_with(b"<expr>") {
                    arg = skipwhite(arg.add(6));
                    continue;
                }
                break;
            }
            (*xp).xp_pattern = arg;
        }

        ptr::null_mut()
    }
}

/// Find all mapping/abbreviation names that match regexp "regmatch".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ExpandMappings(
    pat: *mut c_char,
    regmatch: *mut regmatch_T,
    numMatches: *mut c_int,
    matches: *mut *mut *mut c_char,
) -> c_int {
    unsafe {
        let fuzzy = cmdline_fuzzy_complete(pat);

        *numMatches = 0;
        *matches = ptr::null_mut();

        let mut ga: garray_T = std::mem::zeroed();
        if !fuzzy {
            ga_init(&mut ga, std::mem::size_of::<*mut c_char>() as c_int, 3);
        } else {
            ga_init(&mut ga, std::mem::size_of::<fuzmatch_str_T>() as c_int, 3);
        }

        for i in 0..7 {
            let p: *const c_char = match i {
                0 => c"<silent>".as_ptr(),
                1 => c"<unique>".as_ptr(),
                2 => c"<script>".as_ptr(),
                3 => c"<expr>".as_ptr(),
                4 if !expand_buffer => c"<buffer>".as_ptr(),
                5 => c"<nowait>".as_ptr(),
                6 => c"<special>".as_ptr(),
                _ => continue,
            };

            let mut score: c_int = 0;
            let match_found = if !fuzzy {
                vim_regexec(regmatch, p, 0)
            } else {
                score = fuzzy_match_str(p as *mut c_char, pat);
                score != FUZZY_SCORE_NONE
            };

            if !match_found {
                continue;
            }

            if fuzzy {
                let idx = ga.ga_len;
                let slot = ga_append_via_ptr(&mut ga, std::mem::size_of::<fuzmatch_str_T>())
                    .cast::<fuzmatch_str_T>();
                *slot = fuzmatch_str_T {
                    idx,
                    str_ptr: xstrdup(p),
                    score,
                };
            } else {
                let slot = ga_append_via_ptr(&mut ga, std::mem::size_of::<*mut c_char>())
                    .cast::<*mut c_char>();
                *slot = xstrdup(p);
            }
        }

        for hash in 0..256 {
            let mut mp = if expand_isabbrev {
                if hash > 0 {
                    break;
                }
                first_abbr
            } else if expand_buffer {
                (*curbuf).b_maphash[hash].cast::<mapblock_T>()
            } else {
                maphash[hash]
            };
            while !mp.is_null() {
                if (*mp).m_simplified != 0 || ((*mp).m_mode & expand_mapmodes) == 0 {
                    mp = (*mp).m_next;
                    continue;
                }

                let p = translate_mapping((*mp).m_keys, p_cpo);
                if p.is_null() {
                    mp = (*mp).m_next;
                    continue;
                }

                let mut score: c_int = 0;
                let match_found = if !fuzzy {
                    vim_regexec(regmatch, p, 0)
                } else {
                    score = fuzzy_match_str(p, pat);
                    score != FUZZY_SCORE_NONE
                };

                if !match_found {
                    xfree(p.cast::<c_void>());
                    mp = (*mp).m_next;
                    continue;
                }

                if fuzzy {
                    let idx = ga.ga_len;
                    let slot = ga_append_via_ptr(&mut ga, std::mem::size_of::<fuzmatch_str_T>())
                        .cast::<fuzmatch_str_T>();
                    *slot = fuzmatch_str_T {
                        idx,
                        str_ptr: p,
                        score,
                    };
                } else {
                    let slot = ga_append_via_ptr(&mut ga, std::mem::size_of::<*mut c_char>())
                        .cast::<*mut c_char>();
                    *slot = p;
                }
                mp = (*mp).m_next;
            }
        }

        if ga.ga_len == 0 {
            return FAIL;
        }

        if !fuzzy {
            *matches = ga.ga_data.cast::<*mut c_char>();
            *numMatches = ga.ga_len;
        } else {
            fuzzymatches_to_strmatches(
                ga.ga_data.cast::<fuzmatch_str_T>(),
                matches,
                ga.ga_len,
                false,
            );
            *numMatches = ga.ga_len;
        }

        let mut count = *numMatches;
        if count > 1 {
            if !fuzzy {
                sort_strings(*matches, count);
            }

            let mut ptr1 = *matches;
            let mut ptr2 = ptr1.add(1);
            let ptr3 = ptr1.add(count as usize);

            while (ptr2 as usize) < (ptr3 as usize) {
                if CStr::from_ptr(*ptr1).to_bytes() != CStr::from_ptr(*ptr2).to_bytes() {
                    ptr1 = ptr1.add(1);
                    *ptr1 = *ptr2;
                    ptr2 = ptr2.add(1);
                } else {
                    xfree((*ptr2).cast::<c_void>());
                    ptr2 = ptr2.add(1);
                    count -= 1;
                }
            }
        }

        *numMatches = count;
        if count == 0 { FAIL } else { OK }
    }
}

/// Check for an abbreviation.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_abbr(
    mut c: c_int,
    mut ptr_arg: *mut c_char,
    col: c_int,
    mincol: c_int,
) -> bool {
    unsafe {
        let mut tb = [0u8; MB_MAXBYTES + 4];

        if typebuf.tb_no_abbr_cnt != 0 {
            return false;
        }

        if noremap_keys() && c != Ctrl_RSB {
            return false;
        }

        if col == 0 {
            return false;
        }

        let mut clen: c_int;
        let mut scol: c_int;
        {
            let mut is_id = true;
            let mut p = mb_prevptr(ptr_arg, ptr_arg.add(col as usize));
            let vim_abbr = if !vim_iswordp(p) {
                true
            } else {
                if (p as usize) > (ptr_arg as usize) {
                    is_id = vim_iswordp(mb_prevptr(ptr_arg, p));
                }
                false
            };
            clen = 1;
            let min_ptr = ptr_arg.add(mincol.max(0) as usize);
            while (p as usize) > (min_ptr as usize) {
                p = mb_prevptr(ptr_arg, p);
                if ascii_isspace(*p as u8 as c_int) || (!vim_abbr && is_id != vim_iswordp(p)) {
                    p = p.add(utfc_ptr2len(p) as usize);
                    break;
                }
                clen += 1;
            }
            scol = ((p as isize) - (ptr_arg as isize)) as c_int;
        }

        if scol < mincol {
            scol = mincol;
        }
        if scol < col {
            ptr_arg = ptr_arg.add(scol as usize);
            let mut len = col - scol;
            let mut mp = (*curbuf).b_first_abbr.cast::<mapblock_T>();
            let mut mp2 = first_abbr;
            if mp.is_null() {
                mp = mp2;
                mp2 = ptr::null_mut();
            }
            while !mp.is_null() {
                let mut qlen = (*mp).m_keylen;
                let mut q = (*mp).m_keys;

                if CStr::from_ptr((*mp).m_keys).to_bytes().contains(&K_SPECIAL) {
                    q = xstrdup((*mp).m_keys);
                    qlen = vim_unescape_ks(q) as c_int;
                }
                let match_found = ((*mp).m_mode & State) != 0
                    && qlen == len
                    && std::slice::from_raw_parts(q.cast::<u8>(), len as usize)
                        == std::slice::from_raw_parts(ptr_arg.cast::<u8>(), len as usize);
                if !ptr::eq(q, (*mp).m_keys) {
                    xfree(q.cast::<c_void>());
                }
                if match_found {
                    break;
                }
                if (*mp).m_next.is_null() {
                    mp = mp2;
                    mp2 = ptr::null_mut();
                } else {
                    mp = (*mp).m_next;
                }
            }
            if !mp.is_null() {
                let mut j: usize = 0;
                if c != Ctrl_RSB {
                    if is_special(c) || c == K_SPECIAL as c_int {
                        tb[j] = K_SPECIAL;
                        j += 1;
                        tb[j] = k_second(c);
                        j += 1;
                        tb[j] = k_third(c);
                        j += 1;
                    } else {
                        if c < ABBR_OFF && (c < b' ' as c_int || c > b'~' as c_int) {
                            tb[j] = Ctrl_V as u8;
                            j += 1;
                        }
                        if c >= ABBR_OFF {
                            c -= ABBR_OFF;
                        }
                        let mut newlen =
                            utf_char2bytes(c, tb.as_mut_ptr().add(j).cast::<c_char>()) as usize;
                        tb[j + newlen] = 0;
                        let escaped =
                            vim_strsave_escape_ks(tb.as_mut_ptr().add(j).cast::<c_char>());
                        if !escaped.is_null() {
                            newlen = CStr::from_ptr(escaped).to_bytes().len();
                            ptr::copy(escaped.cast::<u8>(), tb.as_mut_ptr().add(j), newlen);
                            j += newlen;
                            xfree(escaped.cast::<c_void>());
                        }
                    }
                    tb[j] = 0;
                    ins_typebuf(
                        tb.as_mut_ptr().cast::<c_char>(),
                        1,
                        0,
                        true,
                        (*mp).m_silent != 0,
                    );
                }

                let noremap = (*mp).m_noremap;
                let silent = (*mp).m_silent != 0;
                let expr = (*mp).m_expr != 0;

                let s = if expr {
                    eval_map_expr(mp, c)
                } else {
                    (*mp).m_str
                };
                if !s.is_null() {
                    ins_typebuf(s, noremap, 0, true, silent);
                    typebuf.tb_no_abbr_cnt += (CStr::from_ptr(s).to_bytes().len() + j + 1) as c_int;
                    if expr {
                        xfree(s.cast::<c_void>());
                    }
                }

                tb[0] = Ctrl_H;
                tb[1] = 0;
                len = clen;
                while len > 0 {
                    len -= 1;
                    ins_typebuf(tb.as_mut_ptr().cast::<c_char>(), 1, 0, true, silent);
                }
                return true;
            }
        }
        false
    }
}

/// Evaluate the RHS of a mapping or abbreviations and take care of escaping
/// special characters.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn eval_map_expr(mp: *mut mapblock_T, c: c_int) -> *mut c_char {
    unsafe {
        let mut p: *mut c_char = ptr::null_mut();
        let mut expr: *mut c_char = ptr::null_mut();

        if (*mp).m_luaref == LUA_NOREF {
            expr = xstrdup((*mp).m_str);
            vim_unescape_ks(expr);
        }

        let replace_keycodes = (*mp).m_replace_keycodes;

        expr_map_lock += 1;
        set_vim_var_char(c);
        let save_cursor = (*curwin).w_cursor;
        let save_msg_col = msg_col;
        let save_msg_row = msg_row;
        if (*mp).m_luaref != LUA_NOREF {
            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };
            let args = Array {
                items: ptr::null_mut(),
                size: 0,
                capacity: 0,
            };
            let ret = nlua_call_ref(
                (*mp).m_luaref,
                ptr::null(),
                args,
                kRetObject,
                ptr::null_mut(),
                &mut err,
            );
            if ret.obj_type == ObjectType::kObjectTypeString {
                p = string_to_cstr(ret.data.string);
            }
            api_free_object(ret);
            if err.err_type != ErrorType::kErrorTypeNone {
                semsg_multiline!(c"emsg".as_ptr(), c"E5108: %s".as_ptr(), err.msg);
                api_clear_error(&mut err);
            }
        } else {
            p = eval_to_string(expr, false, false);
            xfree(expr.cast::<c_void>());
        }
        expr_map_lock -= 1;
        (*curwin).w_cursor = save_cursor;
        msg_col = save_msg_col;
        msg_row = save_msg_row;

        if p.is_null() {
            return ptr::null_mut();
        }

        let mut res: *mut c_char = ptr::null_mut();
        if replace_keycodes {
            replace_termcodes(
                p,
                CStr::from_ptr(p).to_bytes().len(),
                &mut res,
                0,
                REPTERM_DO_LT,
                ptr::null_mut(),
                p_cpo,
            );
        } else {
            res = vim_strsave_escape_ks(p);
        }
        xfree(p.cast::<c_void>());

        res
    }
}

/// Write map commands for the current mappings to an .exrc file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn makemap(fd: *mut FILE, buf: *mut buf_T) -> c_int {
    unsafe {
        let mut did_cpo = false;

        for abbr in 0..2 {
            for hash in 0..256 {
                let mut mp = if abbr != 0 {
                    if hash > 0 {
                        break;
                    }
                    if !buf.is_null() {
                        (*buf).b_first_abbr.cast::<mapblock_T>()
                    } else {
                        first_abbr
                    }
                } else if !buf.is_null() {
                    (*buf).b_maphash[hash].cast::<mapblock_T>()
                } else {
                    maphash[hash]
                };

                while !mp.is_null() {
                    if (*mp).m_simplified != 0
                        || (*mp).m_noremap == REMAP_SCRIPT
                        || (*mp).m_luaref != LUA_NOREF
                    {
                        mp = (*mp).m_next;
                        continue;
                    }

                    let mut p = (*mp).m_str;
                    while *p != NUL {
                        if *p as u8 == K_SPECIAL
                            && *p.add(1) as u8 == KS_EXTRA
                            && *p.add(2) as u8 == KE_SNR
                        {
                            break;
                        }
                        p = p.add(1);
                    }
                    if *p != NUL {
                        mp = (*mp).m_next;
                        continue;
                    }

                    let mut c1: c_char = NUL;
                    let mut c2: c_char = NUL;
                    let mut c3: c_char = NUL;
                    let mut cmd: *const c_char = if abbr != 0 {
                        c"abbr".as_ptr()
                    } else {
                        c"map".as_ptr()
                    };

                    const MODE_NVSO: c_int =
                        MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING;
                    const MODE_NV: c_int = MODE_NORMAL | MODE_VISUAL;
                    const MODE_NS: c_int = MODE_NORMAL | MODE_SELECT;
                    const MODE_NO: c_int = MODE_NORMAL | MODE_OP_PENDING;
                    const MODE_VS: c_int = MODE_VISUAL | MODE_SELECT;
                    const MODE_VO: c_int = MODE_VISUAL | MODE_OP_PENDING;
                    const MODE_SO: c_int = MODE_SELECT | MODE_OP_PENDING;
                    const MODE_NVS: c_int = MODE_NORMAL | MODE_VISUAL | MODE_SELECT;
                    const MODE_NVO: c_int = MODE_NORMAL | MODE_VISUAL | MODE_OP_PENDING;
                    const MODE_NSO: c_int = MODE_NORMAL | MODE_SELECT | MODE_OP_PENDING;
                    const MODE_VSO: c_int = MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING;
                    const MODE_CI: c_int = MODE_CMDLINE | MODE_INSERT;

                    match (*mp).m_mode {
                        MODE_NVSO => {}
                        MODE_NORMAL => c1 = b'n' as c_char,
                        MODE_VISUAL => c1 = b'x' as c_char,
                        MODE_SELECT => c1 = b's' as c_char,
                        MODE_OP_PENDING => c1 = b'o' as c_char,
                        MODE_NV => {
                            c1 = b'n' as c_char;
                            c2 = b'x' as c_char;
                        }
                        MODE_NS => {
                            c1 = b'n' as c_char;
                            c2 = b's' as c_char;
                        }
                        MODE_NO => {
                            c1 = b'n' as c_char;
                            c2 = b'o' as c_char;
                        }
                        MODE_VS => c1 = b'v' as c_char,
                        MODE_VO => {
                            c1 = b'x' as c_char;
                            c2 = b'o' as c_char;
                        }
                        MODE_SO => {
                            c1 = b's' as c_char;
                            c2 = b'o' as c_char;
                        }
                        MODE_NVS => {
                            c1 = b'n' as c_char;
                            c2 = b'v' as c_char;
                        }
                        MODE_NVO => {
                            c1 = b'n' as c_char;
                            c2 = b'x' as c_char;
                            c3 = b'o' as c_char;
                        }
                        MODE_NSO => {
                            c1 = b'n' as c_char;
                            c2 = b's' as c_char;
                            c3 = b'o' as c_char;
                        }
                        MODE_VSO => {
                            c1 = b'v' as c_char;
                            c2 = b'o' as c_char;
                        }
                        MODE_CI => {
                            if abbr == 0 {
                                cmd = c"map!".as_ptr();
                            }
                        }
                        MODE_CMDLINE => c1 = b'c' as c_char,
                        MODE_INSERT => c1 = b'i' as c_char,
                        MODE_LANGMAP => c1 = b'l' as c_char,
                        MODE_TERMINAL => c1 = b't' as c_char,
                        _ => {
                            iemsg(gettext(c"E228: makemap: Illegal mode".as_ptr()));
                            return FAIL;
                        }
                    }

                    loop {
                        if !did_cpo {
                            if *(*mp).m_str == NUL {
                                did_cpo = true;
                            } else {
                                let specials = [K_SPECIAL as c_char, NL as c_char, NUL];
                                if !strpbrk((*mp).m_str, specials.as_ptr()).is_null()
                                    || !strpbrk((*mp).m_keys, specials.as_ptr()).is_null()
                                {
                                    did_cpo = true;
                                }
                            }
                            if did_cpo
                                && (fputs(c"let s:cpo_save=&cpo".as_ptr(), fd) < 0
                                    || put_eol(fd) == FAIL
                                    || fputs(c"set cpo&vim".as_ptr(), fd) < 0
                                    || put_eol(fd) == FAIL)
                            {
                                return FAIL;
                            }
                        }
                        if c1 != NUL && fputc(c1 as u8 as c_int, fd) < 0 {
                            return FAIL;
                        }
                        if (*mp).m_noremap != REMAP_YES && fputs(c"nore".as_ptr(), fd) < 0 {
                            return FAIL;
                        }
                        if fputs(cmd, fd) < 0 {
                            return FAIL;
                        }
                        if !buf.is_null() && fputs(c" <buffer>".as_ptr(), fd) < 0 {
                            return FAIL;
                        }
                        if (*mp).m_nowait != 0 && fputs(c" <nowait>".as_ptr(), fd) < 0 {
                            return FAIL;
                        }
                        if (*mp).m_silent != 0 && fputs(c" <silent>".as_ptr(), fd) < 0 {
                            return FAIL;
                        }
                        if (*mp).m_expr != 0 && fputs(c" <expr>".as_ptr(), fd) < 0 {
                            return FAIL;
                        }

                        if fputc(b' ' as c_int, fd) < 0
                            || put_escstr(fd, (*mp).m_keys, 0) == FAIL
                            || fputc(b' ' as c_int, fd) < 0
                            || put_escstr(fd, (*mp).m_str, 1) == FAIL
                            || put_eol(fd) == FAIL
                        {
                            return FAIL;
                        }
                        c1 = c2;
                        c2 = c3;
                        c3 = NUL;
                        if c1 == NUL {
                            break;
                        }
                    }

                    mp = (*mp).m_next;
                }
            }
        }
        if did_cpo
            && (fputs(c"let &cpo=s:cpo_save".as_ptr(), fd) < 0
                || put_eol(fd) == FAIL
                || fputs(c"unlet s:cpo_save".as_ptr(), fd) < 0
                || put_eol(fd) == FAIL)
        {
            return FAIL;
        }
        OK
    }
}

/// Write escape string to file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn put_escstr(fd: *mut FILE, strstart: *const c_char, what: c_int) -> c_int {
    unsafe {
        let mut str_ptr = strstart.cast::<u8>();

        if *str_ptr == 0 && what == 1 {
            if fputs(c"<Nop>".as_ptr(), fd) < 0 {
                return FAIL;
            }
            return OK;
        }

        while *str_ptr != 0 {
            let mut p_in = str_ptr.cast::<c_char>();
            let mut p = mb_unescape(&mut p_in);
            str_ptr = p_in.cast::<u8>();
            if !p.is_null() {
                while *p != NUL {
                    if fputc(*p as u8 as c_int, fd) < 0 {
                        return FAIL;
                    }
                    p = p.add(1);
                }
                continue;
            }

            let mut c = *str_ptr as c_int;
            if c == K_SPECIAL as c_int && what != 2 {
                let mut modifiers: c_int = 0;
                if *str_ptr.add(1) == KS_MODIFIER {
                    modifiers = *str_ptr.add(2) as c_int;
                    str_ptr = str_ptr.add(3);

                    let mut mod_ptr = str_ptr.cast::<c_char>();
                    p = mb_unescape(&mut mod_ptr);
                    str_ptr = mod_ptr.cast::<u8>();

                    if p.is_null() {
                        c = *str_ptr as c_int;
                    } else {
                        c = utf_ptr2char(p);
                        str_ptr = str_ptr.sub(1);
                    }
                }
                if c == K_SPECIAL as c_int {
                    c = to_special(*str_ptr.add(1), *str_ptr.add(2));
                    str_ptr = str_ptr.add(2);
                }
                if is_special(c) || modifiers != 0 {
                    if fputs(get_special_key_name(c, modifiers), fd) < 0 {
                        return FAIL;
                    }
                    str_ptr = str_ptr.add(1);
                    continue;
                }
            }

            if c == NL {
                if what == 2 {
                    if fputs(c"\\\x16\n".as_ptr(), fd) < 0 {
                        return FAIL;
                    }
                } else if fputs(c"<NL>".as_ptr(), fd) < 0 {
                    return FAIL;
                }
                str_ptr = str_ptr.add(1);
                continue;
            }

            if what == 2 && (ascii_iswhite(c) || c == b'"' as c_int || c == b'\\' as c_int) {
                if fputc(b'\\' as c_int, fd) < 0 {
                    return FAIL;
                }
            } else if (c < b' ' as c_int
                || c > b'~' as c_int
                || c == b'|' as c_int
                || (what == 0 && c == b' ' as c_int)
                || (what == 1 && ptr::eq(str_ptr, strstart.cast::<u8>()) && c == b' ' as c_int)
                || (what != 2 && c == b'<' as c_int))
                && fputc(Ctrl_V, fd) < 0
            {
                return FAIL;
            }
            if fputc(c, fd) < 0 {
                return FAIL;
            }
            str_ptr = str_ptr.add(1);
        }
        OK
    }
}

/// Check the string "keys" against the lhs of all mappings.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_map(
    keys: *mut c_char,
    mode: c_int,
    exact: c_int,
    ign_mod: c_int,
    abbr: c_int,
    mp_ptr: *mut *mut mapblock_T,
    local_ptr: *mut c_int,
    rhs_lua: *mut c_int,
) -> *mut c_char {
    unsafe {
        *rhs_lua = LUA_NOREF;

        let len = CStr::from_ptr(keys).to_bytes().len() as c_int;
        for local in (0..=1).rev() {
            for hash in 0..256 {
                let mut mp = if abbr != 0 {
                    if hash > 0 {
                        break;
                    }
                    if local != 0 {
                        (*curbuf).b_first_abbr.cast::<mapblock_T>()
                    } else {
                        first_abbr
                    }
                } else if local != 0 {
                    (*curbuf).b_maphash[hash].cast::<mapblock_T>()
                } else {
                    maphash[hash]
                };
                while !mp.is_null() {
                    if ((*mp).m_mode & mode) != 0 && (exact == 0 || (*mp).m_keylen == len) {
                        let mut s = (*mp).m_keys;
                        let mut keylen = (*mp).m_keylen;
                        if ign_mod != 0
                            && keylen >= 3
                            && *s as u8 == K_SPECIAL
                            && *s.add(1) as u8 == KS_MODIFIER
                        {
                            s = s.add(3);
                            keylen -= 3;
                        }
                        let minlen = keylen.min(len).max(0) as usize;
                        if std::slice::from_raw_parts(s.cast::<u8>(), minlen)
                            == std::slice::from_raw_parts(keys.cast::<u8>(), minlen)
                        {
                            if !mp_ptr.is_null() {
                                *mp_ptr = mp;
                            }
                            if !local_ptr.is_null() {
                                *local_ptr = local;
                            }
                            *rhs_lua = (*mp).m_luaref;
                            return if (*mp).m_luaref == LUA_NOREF {
                                (*mp).m_str
                            } else {
                                ptr::null_mut()
                            };
                        }
                    }
                    mp = (*mp).m_next;
                }
            }
        }

        ptr::null_mut()
    }
}

/// "hasmapto()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_hasmapto(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let name = tv_get_string(argvars);
        let mut abbr = false;
        let mut buf = [0 as c_char; NUMBUFLEN];
        let mode = if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            c"nvo".as_ptr()
        } else {
            let m = tv_get_string_buf(argvars.add(1), buf.as_mut_ptr());
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                abbr = tv_get_bool(argvars.add(2)) != 0;
            }
            m
        };

        (*rettv).vval.v_number = i64::from(map_to_exists(name, mode, abbr));
    }
}

unsafe fn mapblock_fill_dict(
    mp: *const mapblock_T,
    lhsrawalt: *const c_char,
    buffer_value: c_int,
    abbr: bool,
    compatible: bool,
    arena: *mut Arena,
) -> Dict {
    unsafe {
        let mut dict = arena_dict(arena, 20);
        let lhs = str2special_arena(
            (*mp).m_keys,
            compatible,
            if compatible { kFalse } else { kNone },
            arena,
        );
        let mapmode = arena_alloc(arena, 7, false).cast::<c_char>();
        map_mode_to_chars((*mp).m_mode, mapmode);
        let noremap_value = if compatible {
            i64::from((*mp).m_noremap != 0)
        } else if (*mp).m_noremap == REMAP_SCRIPT {
            2
        } else {
            i64::from((*mp).m_noremap != 0)
        };

        if (*mp).m_luaref != LUA_NOREF {
            put_c(
                &mut dict,
                c"callback",
                luaref_obj(api_new_luaref((*mp).m_luaref)),
            );
        } else {
            let rhs = cstr_as_string(if compatible {
                (*mp).m_orig_str
            } else {
                str2special_arena((*mp).m_str, false, kNone, arena)
            });
            put_c(&mut dict, c"rhs", string_obj(rhs));
        }
        if !(*mp).m_desc.is_null() {
            put_c(&mut dict, c"desc", cstr_as_obj((*mp).m_desc));
        }
        put_c(&mut dict, c"lhs", cstr_as_obj(lhs));
        put_c(&mut dict, c"lhsraw", cstr_as_obj((*mp).m_keys));
        if !lhsrawalt.is_null() {
            put_c(&mut dict, c"lhsrawalt", cstr_as_obj(lhsrawalt));
        }
        put_c(&mut dict, c"noremap", integer_obj(noremap_value));
        put_c(
            &mut dict,
            c"script",
            integer_obj(i64::from((*mp).m_noremap == REMAP_SCRIPT)),
        );
        put_c(
            &mut dict,
            c"expr",
            integer_obj(i64::from((*mp).m_expr != 0)),
        );
        put_c(
            &mut dict,
            c"silent",
            integer_obj(i64::from((*mp).m_silent != 0)),
        );
        put_c(
            &mut dict,
            c"sid",
            integer_obj(i64::from((*mp).m_script_ctx.sc_sid)),
        );
        put_c(&mut dict, c"scriptversion", integer_obj(1));
        put_c(
            &mut dict,
            c"lnum",
            integer_obj(i64::from((*mp).m_script_ctx.sc_lnum)),
        );
        put_c(&mut dict, c"buffer", integer_obj(i64::from(buffer_value)));
        if !compatible {
            put_c(&mut dict, c"buf", integer_obj(i64::from(buffer_value)));
        }
        put_c(
            &mut dict,
            c"nowait",
            integer_obj(i64::from((*mp).m_nowait != 0)),
        );
        put_c(
            &mut dict,
            c"replace_keycodes",
            integer_obj(i64::from((*mp).m_replace_keycodes)),
        );
        put_c(&mut dict, c"mode", cstr_as_obj(mapmode));
        put_c(&mut dict, c"abbr", integer_obj(i64::from(abbr)));
        put_c(
            &mut dict,
            c"mode_bits",
            integer_obj(i64::from((*mp).m_mode)),
        );

        dict
    }
}

unsafe fn get_maparg(argvars: *mut typval_T, rettv: *mut typval_T, exact: c_int) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        let keys = tv_get_string(argvars) as *mut c_char;
        if *keys == NUL {
            return;
        }

        let mut buf = [0 as c_char; NUMBUFLEN];
        let mut abbr = false;
        let mut get_dict = false;

        let which = if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            let w = tv_get_string_buf_chk(argvars.add(1), buf.as_mut_ptr());
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                abbr = tv_get_bool(argvars.add(2)) != 0;
                if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                    get_dict = tv_get_bool(argvars.add(3)) != 0;
                }
            }
            w
        } else {
            c"".as_ptr()
        };
        if which.is_null() {
            return;
        }

        let mut keys_buf: *mut c_char = ptr::null_mut();
        let mut alt_keys_buf: *mut c_char = ptr::null_mut();
        let mut did_simplify = false;
        let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
        let mut which_mut = which as *mut c_char;
        let mode = get_map_mode(&mut which_mut, false);

        let keys_len = CStr::from_ptr(keys).to_bytes().len();
        let keys_simplified = replace_termcodes(
            keys,
            keys_len,
            &mut keys_buf,
            0,
            flags,
            &mut did_simplify,
            p_cpo,
        );
        let mut mp: *mut mapblock_T = ptr::null_mut();
        let mut buffer_local: c_int = 0;
        let mut rhs_lua: LuaRef = LUA_NOREF;
        let mut rhs = check_map(
            keys_simplified,
            mode,
            exact,
            0,
            c_int::from(abbr),
            &mut mp,
            &mut buffer_local,
            &mut rhs_lua,
        );
        if did_simplify {
            replace_termcodes(
                keys,
                keys_len,
                &mut alt_keys_buf,
                0,
                flags | REPTERM_NO_SIMPLIFY,
                ptr::null_mut(),
                p_cpo,
            );
            rhs = check_map(
                alt_keys_buf,
                mode,
                exact,
                0,
                c_int::from(abbr),
                &mut mp,
                &mut buffer_local,
                &mut rhs_lua,
            );
        }

        if !get_dict {
            if !rhs.is_null() {
                if *rhs == NUL {
                    (*rettv).vval.v_string = xstrdup(c"<Nop>".as_ptr());
                } else {
                    (*rettv).vval.v_string = str2special_save(rhs, false, kFalse);
                }
            } else if rhs_lua != LUA_NOREF {
                (*rettv).vval.v_string = nlua_funcref_str((*mp).m_luaref, ptr::null_mut(), true);
            }
        } else if !mp.is_null() && (!rhs.is_null() || rhs_lua != LUA_NOREF) {
            let mut arena = Arena {
                cur_blk: ptr::null_mut(),
                pos: 0,
                size: 0,
            };
            let dict = mapblock_fill_dict(
                mp,
                if did_simplify {
                    keys_simplified
                } else {
                    ptr::null()
                },
                buffer_local,
                abbr,
                true,
                &mut arena,
            );
            let mut dobj = dict_obj(dict);
            object_to_vim_take_luaref(&mut dobj, rettv, true, ptr::null_mut());
            arena_mem_free(arena_finish(&mut arena));
        } else {
            tv_dict_alloc_ret(rettv);
        }

        xfree(keys_buf.cast::<c_void>());
        xfree(alt_keys_buf.cast::<c_void>());
    }
}

unsafe fn get_map_mode_string(mode_string: *const c_char, abbr: bool) -> c_int {
    unsafe {
        let mut p = mode_string;
        const MASK_V: c_int = MODE_VISUAL | MODE_SELECT;
        const MASK_MAP: c_int = MODE_VISUAL | MODE_SELECT | MODE_NORMAL | MODE_OP_PENDING;
        const MASK_BANG: c_int = MODE_INSERT | MODE_CMDLINE;

        if *p == NUL {
            p = c" ".as_ptr();
        }
        let mut mode: c_int = 0;
        while *p != NUL {
            let modec = *p as u8;
            p = p.add(1);
            let tmode = match modec {
                b'i' => MODE_INSERT,
                b'l' => MODE_LANGMAP,
                b'c' => MODE_CMDLINE,
                b'n' => MODE_NORMAL,
                b'x' => MODE_VISUAL,
                b's' => MODE_SELECT,
                b'o' => MODE_OP_PENDING,
                b't' => MODE_TERMINAL,
                b'v' => MASK_V,
                b'!' => MASK_BANG,
                b' ' => MASK_MAP,
                _ => return 0,
            };
            mode |= tmode;
        }
        if (abbr && (mode & !MASK_BANG) != 0)
            || (!abbr
                && (mode & (mode - 1)) != 0
                && !(((mode & MASK_BANG) != 0 && (mode & !MASK_BANG) == 0)
                    || ((mode & MASK_MAP) != 0 && (mode & !MASK_MAP) == 0)))
        {
            return 0;
        }

        mode
    }
}

/// "mapset()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_mapset(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            return;
        }

        let mut buf = [0 as c_char; NUMBUFLEN];
        let dict_only = (*argvars).v_type == VarType::VAR_DICT;

        let (which, is_abbr, d) = if dict_only {
            let d = (*argvars).vval.v_ptr.cast::<dict_T>();
            let which = tv_dict_get_string(d, c"mode".as_ptr(), false);
            let is_abbr = tv_dict_get_bool(d, c"abbr".as_ptr(), -1) as c_int;
            if which.is_null() || is_abbr < 0 {
                emsg(gettext(E_ENTRIES_MISSING_IN_MAPSET_DICT_ARGUMENT.as_ptr()));
                return;
            }
            (which as *const c_char, is_abbr, d)
        } else {
            let which = tv_get_string_buf_chk(argvars, buf.as_mut_ptr());
            if which.is_null() {
                return;
            }
            let is_abbr = tv_get_bool(argvars.add(1)) as c_int;
            if tv_check_for_dict_arg(argvars, 2) == FAIL {
                return;
            }
            let d = (*argvars.add(2)).vval.v_ptr.cast::<dict_T>();
            (which, is_abbr, d)
        };
        let mode = get_map_mode_string(which, is_abbr != 0);
        if mode == 0 {
            semsg!(gettext(E_ILLEGAL_MAP_MODE_STRING_STR.as_ptr()), which);
            return;
        }

        let lhs = tv_dict_get_string(d, c"lhs".as_ptr(), false);
        let lhsraw = tv_dict_get_string(d, c"lhsraw".as_ptr(), false);
        let lhsrawalt = tv_dict_get_string(d, c"lhsrawalt".as_ptr(), false);
        let mut orig_rhs = tv_dict_get_string(d, c"rhs".as_ptr(), false) as *const c_char;
        let mut rhs_lua: LuaRef = LUA_NOREF;
        let callback_di = tv_dict_find(d, c"callback".as_ptr(), 8);
        if !callback_di.is_null() && (*callback_di).di_tv.v_type == VarType::VAR_FUNC {
            let fp = find_func((*callback_di).di_tv.vval.v_string);
            if !fp.is_null() && ((*fp).uf_flags & FC_LUAREF) != 0 {
                rhs_lua = api_new_luaref((*fp).uf_luaref);
                orig_rhs = c"".as_ptr();
            }
        }
        if lhs.is_null() || lhsraw.is_null() || orig_rhs.is_null() {
            emsg(gettext(E_ENTRIES_MISSING_IN_MAPSET_DICT_ARGUMENT.as_ptr()));
            api_free_luaref(rhs_lua);
            return;
        }

        let mut noremap = if tv_dict_get_number(d, c"noremap".as_ptr()) != 0 {
            REMAP_NONE
        } else {
            0
        };
        if tv_dict_get_number(d, c"script".as_ptr()) != 0 {
            noremap = REMAP_SCRIPT;
        }
        let mut args = MapArguments::new();
        args.expr = tv_dict_get_number(d, c"expr".as_ptr()) != 0;
        args.silent = tv_dict_get_number(d, c"silent".as_ptr()) != 0;
        args.nowait = tv_dict_get_number(d, c"nowait".as_ptr()) != 0;
        args.replace_keycodes = tv_dict_get_number(d, c"replace_keycodes".as_ptr()) != 0;
        args.desc = tv_dict_get_string(d, c"desc".as_ptr(), true);

        let sid = tv_dict_get_number(d, c"sid".as_ptr()) as scid_T;
        let lnum = tv_dict_get_number(d, c"lnum".as_ptr()) as linenr_T;
        let buffer = tv_dict_get_number(d, c"buffer".as_ptr()) != 0;

        set_maparg_rhs(
            orig_rhs,
            CStr::from_ptr(orig_rhs).to_bytes().len(),
            rhs_lua,
            sid,
            p_cpo,
            &mut args,
        );

        let map_table = if buffer {
            (&raw mut (*curbuf).b_maphash).cast::<*mut mapblock_T>()
        } else {
            (&raw mut maphash).cast::<*mut mapblock_T>()
        };
        let abbr_table = if buffer {
            (&raw mut (*curbuf).b_first_abbr).cast::<*mut mapblock_T>()
        } else {
            &raw mut first_abbr
        };

        let mut unmap_args = MapArguments::new();
        set_maparg_lhs_rhs(
            lhs,
            CStr::from_ptr(lhs).to_bytes().len(),
            c"".as_ptr(),
            0,
            LUA_NOREF,
            p_cpo,
            &mut unmap_args,
        );
        unmap_args.buffer = buffer;
        buf_do_map(
            MAPTYPE_UNMAP_LHS,
            &mut unmap_args,
            mode,
            is_abbr != 0,
            curbuf,
        );
        xfree(unmap_args.rhs.cast::<c_void>());
        xfree(unmap_args.orig_rhs.cast::<c_void>());

        let mut mp_result: [*mut mapblock_T; 2] = [ptr::null_mut(), ptr::null_mut()];
        mp_result[0] = map_add(
            curbuf,
            map_table,
            abbr_table,
            lhsraw,
            &args,
            noremap,
            mode,
            is_abbr != 0,
            sid,
            lnum,
            false,
        );
        if !lhsrawalt.is_null() {
            mp_result[1] = map_add(
                curbuf,
                map_table,
                abbr_table,
                lhsrawalt,
                &args,
                noremap,
                mode,
                is_abbr != 0,
                sid,
                lnum,
                true,
            );
        }

        if !mp_result[0].is_null() && !mp_result[1].is_null() {
            (*mp_result[0]).m_alt = mp_result[1];
            (*mp_result[1]).m_alt = mp_result[0];
        }
    }
}

/// "maplist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_maplist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
        let abbr = (*argvars).v_type != VarType::VAR_UNKNOWN && tv_get_bool(argvars) != 0;

        tv_list_alloc_ret(rettv, kListLenUnknown);

        for buffer_local in 0..=1 {
            for hash in 0..256 {
                let mut mp = if abbr {
                    if hash > 0 {
                        break;
                    }
                    if buffer_local != 0 {
                        (*curbuf).b_first_abbr.cast::<mapblock_T>()
                    } else {
                        first_abbr
                    }
                } else if buffer_local != 0 {
                    (*curbuf).b_maphash[hash].cast::<mapblock_T>()
                } else {
                    maphash[hash]
                };
                while !mp.is_null() {
                    if (*mp).m_simplified != 0 {
                        mp = (*mp).m_next;
                        continue;
                    }

                    let mut keys_buf: *mut c_char = ptr::null_mut();
                    let mut did_simplify = false;

                    let mut arena = Arena {
                        cur_blk: ptr::null_mut(),
                        pos: 0,
                        size: 0,
                    };
                    let lhs = str2special_arena((*mp).m_keys, true, kFalse, &mut arena);
                    replace_termcodes(
                        lhs,
                        CStr::from_ptr(lhs).to_bytes().len(),
                        &mut keys_buf,
                        0,
                        flags,
                        &mut did_simplify,
                        p_cpo,
                    );

                    let dict = mapblock_fill_dict(
                        mp,
                        if did_simplify { keys_buf } else { ptr::null() },
                        buffer_local,
                        abbr,
                        true,
                        &mut arena,
                    );
                    let mut d = typval_T {
                        v_type: VarType::VAR_UNKNOWN,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval { v_number: 0 },
                    };
                    let mut dobj = dict_obj(dict);
                    object_to_vim_take_luaref(&mut dobj, &mut d, true, ptr::null_mut());
                    debug_assert!(d.v_type == VarType::VAR_DICT);
                    tv_list_append_dict(
                        (*rettv).vval.v_ptr.cast::<list_T>(),
                        d.vval.v_ptr.cast::<dict_T>(),
                    );
                    arena_mem_free(arena_finish(&mut arena));
                    xfree(keys_buf.cast::<c_void>());
                    mp = (*mp).m_next;
                }
            }
        }
    }
}

/// "maparg()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_maparg(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        get_maparg(argvars, rettv, 1);
    }
}

/// "mapcheck()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_mapcheck(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        get_maparg(argvars, rettv, 0);
    }
}

/// Add a mapping.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn add_map(lhs: *mut c_char, rhs: *mut c_char, mode: c_int, buffer: bool) {
    unsafe {
        let mut args = MapArguments::new();
        set_maparg_lhs_rhs(
            lhs,
            CStr::from_ptr(lhs).to_bytes().len(),
            rhs,
            CStr::from_ptr(rhs).to_bytes().len(),
            LUA_NOREF,
            p_cpo,
            &mut args,
        );
        args.buffer = buffer;

        buf_do_map(MAPTYPE_NOREMAP, &mut args, mode, false, curbuf);
        xfree(args.rhs.cast::<c_void>());
        xfree(args.orig_rhs.cast::<c_void>());
    }
}

unsafe fn langmap_set_entry(from: c_int, to: c_int) {
    unsafe {
        let mut entries = langmap_mapga.ga_data.cast::<langmap_entry_T>();
        let mut a: usize = 0;
        debug_assert!(langmap_mapga.ga_len >= 0);
        let mut b = langmap_mapga.ga_len as usize;

        while a != b {
            let i = (a + b) / 2;
            let d = (*entries.add(i)).from - from;

            if d == 0 {
                (*entries.add(i)).to = to;
                return;
            }
            if d < 0 {
                a = i + 1;
            } else {
                b = i;
            }
        }

        ga_grow(&raw mut langmap_mapga, 1);

        entries = langmap_mapga.ga_data.cast::<langmap_entry_T>().add(a);
        ptr::copy(entries, entries.add(1), (langmap_mapga.ga_len as usize) - a);
        langmap_mapga.ga_len += 1;
        (*entries).from = from;
        (*entries).to = to;
    }
}

/// Apply 'langmap' to multi-byte character "c" and return the result.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn langmap_adjust_mb(c: c_int) -> c_int {
    unsafe {
        let entries = langmap_mapga.ga_data.cast::<langmap_entry_T>();
        let mut a: c_int = 0;
        let mut b = langmap_mapga.ga_len;

        while a != b {
            let i = (a + b) / 2;
            let d = (*entries.add(i as usize)).from - c;

            if d == 0 {
                return (*entries.add(i as usize)).to;
            }
            if d < 0 {
                a = i + 1;
            } else {
                b = i;
            }
        }
        c
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn langmap_init() {
    unsafe {
        for i in 0..256 {
            langmap_mapchar[i] = i as u8;
        }
        ga_init(
            &raw mut langmap_mapga,
            std::mem::size_of::<langmap_entry_T>() as c_int,
            8,
        );
    }
}

/// Called when langmap option is set; the language map can be
/// changed at any time!
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn did_set_langmap(args: *mut optset_T) -> *const c_char {
    unsafe {
        ga_clear(&raw mut langmap_mapga);
        langmap_init();

        let mut p = p_langmap;
        while *p != NUL {
            let mut p2 = p;
            while *p2 != NUL && *p2 != b',' as c_char && *p2 != b';' as c_char {
                if *p2 == b'\\' as c_char && *p2.add(1) != NUL {
                    p2 = p2.add(1);
                }
                p2 = p2.add(utfc_ptr2len(p2) as usize);
            }
            if *p2 == b';' as c_char {
                p2 = p2.add(1);
            } else {
                p2 = ptr::null_mut();
            }
            while *p != NUL {
                if *p == b',' as c_char {
                    p = p.add(1);
                    break;
                }
                if *p == b'\\' as c_char && *p.add(1) != NUL {
                    p = p.add(1);
                }
                let from = utf_ptr2char(p);
                let from_ptr = p;
                let mut to: c_int = NUL as c_int;
                let mut to_ptr: *const c_char = c"".as_ptr();
                if p2.is_null() {
                    p = p.add(utfc_ptr2len(p) as usize);
                    if *p != b',' as c_char {
                        if *p == b'\\' as c_char {
                            p = p.add(1);
                        }
                        to_ptr = p;
                        to = utf_ptr2char(p);
                    }
                } else if *p2 != b',' as c_char {
                    if *p2 == b'\\' as c_char {
                        p2 = p2.add(1);
                    }
                    to_ptr = p2;
                    to = utf_ptr2char(p2);
                }
                if to == NUL as c_int {
                    return opt_error(
                        (*args).os_errbuf,
                        c"E357: 'langmap': Matching character missing for %s".as_ptr(),
                        transchar(from),
                    );
                }

                if from >= 256 {
                    langmap_set_entry(from, to);
                } else {
                    if to > UCHAR_MAX {
                        swmsg!(
                            true,
                            c"'langmap': Mapping from %.*s to %.*s will not work properly".as_ptr(),
                            utf_ptr2len(from_ptr),
                            from_ptr,
                            utf_ptr2len(to_ptr),
                            to_ptr,
                        );
                    }
                    langmap_mapchar[(from & 255) as usize] = to as u8;
                }

                p = p.add(utfc_ptr2len(p) as usize);
                if !p2.is_null() {
                    p2 = p2.add(utfc_ptr2len(p2) as usize);
                    if *p == b';' as c_char {
                        p = p2;
                        if *p != NUL {
                            if *p != b',' as c_char {
                                return opt_error(
                                    (*args).os_errbuf,
                                    c"E358: 'langmap': Extra characters after semicolon: %s"
                                        .as_ptr(),
                                    p,
                                );
                            }
                            p = p.add(1);
                        }
                        break;
                    }
                }
            }
        }

        ptr::null()
    }
}

unsafe fn do_exmap(eap: *mut exarg_T, isabbrev: bool) {
    unsafe {
        let mut cmdp = (*eap).cmd;
        let mode = get_map_mode(&mut cmdp, (*eap).forceit != 0 || isabbrev);

        let maptype = if *cmdp == b'n' as c_char {
            MAPTYPE_NOREMAP
        } else if *cmdp == b'u' as c_char {
            MAPTYPE_UNMAP
        } else {
            MAPTYPE_MAP
        };
        let mut parsed_args = MapArguments::new();
        let result = str_to_mapargs((*eap).arg, maptype == MAPTYPE_UNMAP, &mut parsed_args);
        match result {
            0 => match buf_do_map(maptype, &mut parsed_args, mode, isabbrev, curbuf) {
                1 => {
                    emsg(gettext(e_invarg.as_ptr()));
                }
                2 => {
                    emsg(if isabbrev {
                        gettext(e_noabbr.as_ptr())
                    } else {
                        gettext(e_nomap.as_ptr())
                    });
                }
                5 => {
                    semsg!(
                        if isabbrev {
                            gettext(E_ABBREVIATION_ALREADY_EXISTS_FOR_STR.as_ptr())
                        } else {
                            gettext(E_MAPPING_ALREADY_EXISTS_FOR_STR.as_ptr())
                        },
                        parsed_args.lhs.as_ptr(),
                    );
                }
                6 => {
                    semsg!(
                        if isabbrev {
                            gettext(E_GLOBAL_ABBREVIATION_ALREADY_EXISTS_FOR_STR.as_ptr())
                        } else {
                            gettext(E_GLOBAL_MAPPING_ALREADY_EXISTS_FOR_STR.as_ptr())
                        },
                        parsed_args.lhs.as_ptr(),
                    );
                }
                _ => {}
            },
            1 => {
                emsg(gettext(e_invarg.as_ptr()));
            }
            _ => {
                debug_assert!(false, "Unknown return code from str_to_mapargs!");
            }
        }
        xfree(parsed_args.rhs.cast::<c_void>());
        xfree(parsed_args.orig_rhs.cast::<c_void>());
    }
}

/// ":abbreviate" and friends.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_abbreviate(eap: *mut exarg_T) {
    unsafe {
        do_exmap(eap, true);
    }
}

/// ":map" and friends.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_map(eap: *mut exarg_T) {
    unsafe {
        if secure != 0 {
            secure = 2;
            msg_outtrans((*eap).cmd, 0, false);
            msg_putchar(b'\n' as c_int);
        }
        do_exmap(eap, false);
    }
}

/// ":unmap" and friends.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_unmap(eap: *mut exarg_T) {
    unsafe {
        do_exmap(eap, false);
    }
}

/// ":mapclear" and friends.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_mapclear(eap: *mut exarg_T) {
    unsafe {
        do_mapclear((*eap).cmd, (*eap).arg, (*eap).forceit, 0);
    }
}

/// ":abclear" and friends.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_abclear(eap: *mut exarg_T) {
    unsafe {
        do_mapclear((*eap).cmd, (*eap).arg, 1, 1);
    }
}

/// Set, tweak, or remove a mapping in a mode.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn modify_keymap(
    channel_id: u64,
    mut buffer: Buffer,
    mut maptype: c_int,
    mode: NvimString,
    lhs: NvimString,
    rhs: NvimString,
    opts: *mut KeyDict_keymap,
    err: *mut Error,
) {
    unsafe {
        let mut lua_funcref: LuaRef = LUA_NOREF;
        let is_unmap = maptype == MAPTYPE_UNMAP || maptype == MAPTYPE_UNMAP_LHS;
        let global = buffer == -1;
        if global {
            buffer = 0;
        }
        let target_buf = find_buffer_by_handle(buffer, err);
        if target_buf.is_null() {
            return;
        }

        let save_current_sctx = api_set_sctx(channel_id);

        let mut parsed_args = MapArguments::new();
        if !opts.is_null() {
            parsed_args.nowait = (*opts).nowait;
            parsed_args.noremap = (*opts).noremap;
            parsed_args.silent = (*opts).silent;
            parsed_args.script = (*opts).script;
            parsed_args.expr = (*opts).expr;
            parsed_args.unique = (*opts).unique;
            parsed_args.replace_keycodes = (*opts).replace_keycodes;
            if has_key((*opts).is_set__keymap_, KEYSET_OPTIDX_keymap__callback) {
                lua_funcref = (*opts).callback;
                (*opts).callback = LUA_NOREF;
            }
            if has_key((*opts).is_set__keymap_, KEYSET_OPTIDX_keymap__desc) {
                parsed_args.desc = string_to_cstr((*opts).desc);
            }
        }
        parsed_args.buffer = !global;

        'block: {
            if parsed_args.replace_keycodes && !parsed_args.expr {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"\"replace_keycodes\" requires \"expr\"".as_ptr(),
                );
                break 'block;
            }

            if !set_maparg_lhs_rhs(
                lhs.data,
                lhs.size,
                rhs.data,
                rhs.size,
                lua_funcref,
                p_cpo,
                &mut parsed_args,
            ) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"LHS exceeds maximum map length: %s".as_ptr(),
                    lhs.data,
                );
                break 'block;
            }

            if parsed_args.lhs_len > MAXMAPLEN || parsed_args.alt_lhs_len > MAXMAPLEN {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"LHS exceeds maximum map length: %s".as_ptr(),
                    lhs.data,
                );
                break 'block;
            }

            let mut p: *mut c_char = if mode.size > 0 {
                mode.data
            } else {
                c"m".as_ptr() as *mut c_char
            };
            let forceit = *p == b'!' as c_char;
            let mode_val = get_map_mode(&mut p, forceit);
            if forceit {
                debug_assert!(ptr::eq(p, mode.data));
                p = p.add(1);
            }
            let is_abbrev = (mode_val & (MODE_INSERT | MODE_CMDLINE)) != 0 && *p == b'a' as c_char;
            if is_abbrev {
                p = p.add(1);
            }
            if mode.size > 0 && ((p as usize) - (mode.data as usize)) != mode.size {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Invalid mode shortname: \"%s\"".as_ptr(),
                    mode.data,
                );
                break 'block;
            }

            if parsed_args.lhs_len == 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Invalid (empty) LHS".as_ptr(),
                );
                break 'block;
            }

            let is_noremap = parsed_args.noremap;
            debug_assert!(!(is_unmap && is_noremap));

            if !is_unmap
                && lua_funcref == LUA_NOREF
                && parsed_args.rhs_len == 0
                && !parsed_args.rhs_is_noop
            {
                if rhs.size == 0 {
                    parsed_args.rhs_is_noop = true;
                } else {
                    std::process::abort();
                }
            } else if is_unmap && (parsed_args.rhs_len != 0 || parsed_args.rhs_lua != LUA_NOREF) {
                if parsed_args.rhs_len != 0 {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Gave nonempty RHS in unmap command: %s".as_ptr(),
                        parsed_args.rhs,
                    );
                } else {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Gave nonempty RHS for unmap".as_ptr(),
                    );
                }
                break 'block;
            }

            if maptype == MAPTYPE_MAP && is_noremap {
                maptype = MAPTYPE_NOREMAP;
            }

            match buf_do_map(maptype, &mut parsed_args, mode_val, is_abbrev, target_buf) {
                0 => {}
                1 => {
                    api_set_error!(err, ErrorType::kErrorTypeException, e_invarg.as_ptr(), 0);
                }
                2 => {
                    api_set_error!(err, ErrorType::kErrorTypeException, e_nomap.as_ptr(), 0);
                }
                5 => {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        if is_abbrev {
                            E_ABBREVIATION_ALREADY_EXISTS_FOR_STR.as_ptr()
                        } else {
                            E_MAPPING_ALREADY_EXISTS_FOR_STR.as_ptr()
                        },
                        lhs.data,
                    );
                }
                6 => {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        if is_abbrev {
                            E_GLOBAL_ABBREVIATION_ALREADY_EXISTS_FOR_STR.as_ptr()
                        } else {
                            E_GLOBAL_MAPPING_ALREADY_EXISTS_FOR_STR.as_ptr()
                        },
                        lhs.data,
                    );
                }
                _ => {
                    debug_assert!(false, "Unrecognized return code!");
                }
            }
        }

        current_sctx = save_current_sctx;
        nlua_clear_ref(&mut parsed_args.rhs_lua);
        xfree(parsed_args.rhs.cast::<c_void>());
        xfree(parsed_args.orig_rhs.cast::<c_void>());
        xfree(parsed_args.desc.cast::<c_void>());
    }
}

/// Get an array containing dictionaries describing mappings
/// based on mode and buffer id.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn keymap_array(
    mode: NvimString,
    buf: *mut buf_T,
    arena: *mut Arena,
) -> Array {
    unsafe {
        let mut mappings: ArrayBuilder = std::mem::zeroed();
        mappings.init();

        let mut p: *mut c_char = if mode.size > 0 {
            mode.data
        } else {
            c"m".as_ptr() as *mut c_char
        };
        let forceit = *p == b'!' as c_char;
        let int_mode = get_map_mode(&mut p, forceit);
        if forceit {
            debug_assert!(ptr::eq(p, mode.data));
            p = p.add(1);
        }
        let is_abbrev = (int_mode & (MODE_INSERT | MODE_CMDLINE)) != 0 && *p == b'a' as c_char;

        let buffer_value = if buf.is_null() { 0 } else { (*buf).b_fnum };

        let limit = if is_abbrev { 1 } else { MAX_MAPHASH };
        for i in 0..limit {
            let mut current_maphash: *const mapblock_T = if is_abbrev {
                if !buf.is_null() {
                    (*buf).b_first_abbr.cast::<mapblock_T>()
                } else {
                    first_abbr
                }
            } else if !buf.is_null() {
                (*buf).b_maphash[i].cast::<mapblock_T>()
            } else {
                maphash[i]
            };
            while !current_maphash.is_null() {
                if (*current_maphash).m_simplified == 0
                    && (int_mode & (*current_maphash).m_mode) != 0
                {
                    let alt_keys = if !(*current_maphash).m_alt.is_null() {
                        (*(*current_maphash).m_alt).m_keys as *const c_char
                    } else {
                        ptr::null()
                    };
                    mappings.push(dict_obj(mapblock_fill_dict(
                        current_maphash,
                        alt_keys,
                        buffer_value,
                        is_abbrev,
                        false,
                        arena,
                    )));
                }
                current_maphash = (*current_maphash).m_next;
            }
        }

        arena_take_arraybuilder(arena, &mut mappings)
    }
}
