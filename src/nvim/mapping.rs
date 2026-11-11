#![forbid(unsafe_code)]

//! Code for mappings and abbreviations (`src/nvim/mapping.c`).

use crate::api::buffer::KeyDict_keymap;
use crate::api::private::converter::object_to_vim_take_luaref;
use crate::api::private::helpers::{
    ArrayBuilder, api_clear_error, api_free_object, api_set_error, api_set_sctx, arena_dict,
    arena_take_arraybuilder, cstr_to_arena_obj, dict_obj, has_key, integer_obj, luaref_obj, put_c,
    string_obj, string_to_cstr,
};
use crate::api::private::validate::{Array, Dict, Error, ErrorType, NvimString, ObjectType};
use crate::api::tabpage::Buffer;
use crate::buffer::FileBuf;
use crate::charset::{
    NUL, ascii_isspace, ascii_iswhite, is_special, to_special, transchar, vim_iswordp,
};
use crate::cmdexpand::{EXPAND_MAPPINGS, EXPAND_NOTHING, MB_MAXBYTES, cmdline_fuzzy_complete};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, TypVal, TypvalVval, VAR_UNLOCKED, VarType, nlua_clear_ref, tv_check_for_dict_arg,
    tv_dict_alloc_ret, tv_dict_find, tv_dict_get_bool, tv_dict_get_number, tv_dict_get_string,
    tv_get_bool, tv_get_string, tv_get_string_buf, tv_get_string_buf_chk, tv_list_alloc_ret,
    tv_list_append_dict,
};
use crate::eval::userfunc::{FC_LUAREF, find_func};
use crate::eval::vars::{Scid, set_vim_var_char};
use crate::eval::{K_LIST_LEN_UNKNOWN_CONST, eval_to_string, last_set_msg};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{CTRL_C_CONST, Exarg};
use crate::ex_session::put_eol;
use crate::fuzzy::{
    FUZZY_SCORE_NONE, FuzmatchStr, fuzzy_match_str, fuzzymatches_slice_to_strmatches,
};
use crate::globals::*;
use crate::input::{REMAP_SCRIPT, ins_typebuf, noremap_keys};
use crate::keycodes::{
    K_SPECIAL, K_ZERO, KE_LUA, KE_SNR, KS_EXTRA, KS_MODIFIER, NL, REPTERM_DO_LT, REPTERM_FROM_PART,
    REPTERM_NO_SIMPLIFY, get_special_key_name, replace_termcodes, vim_strsave_escape_ks,
    vim_unescape_ks,
};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{
    K_RET_OBJECT, api_free_luaref, api_new_luaref, nlua_call_ref, nlua_funcref_str, nlua_set_sctx,
};
use crate::mbyte::{
    mb_unescape, utf_char2bytes, utf_ptr2char, utf_ptr2len, utf_slice2char, utfc_ptr2len,
};
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::message::{
    emsg, iemsg, message_filtered, msg, msg_clr_eos, msg_ext_set_kind, msg_outtrans,
    msg_outtrans_special, msg_putchar, msg_puts, msg_puts_hl, msg_start, semsg, semsg_multiline,
    str2special_save, swmsg,
};
use crate::option::opt_error;
use crate::option_vars::*;
use crate::optionstr::Optset;
use crate::os::env::ExpandCtx;
use crate::os::fs::CFile;
use crate::regexp::{CTRL_V_CONST, NUMBUFLEN, Regmatch, vim_regexec};
use crate::runtime::{Sctx, sourcing_lnum};
use crate::strings::{format_to_buf, gettext, sort_strings};
use crate::viml::parser::expressions::CTRL_H_CONST;
use std::ffi::{CStr, CString};

use crate::cursor::Linenr;

pub const MAXMAPLEN: usize = 50;
pub const MAX_MAPHASH: usize = 256;

pub(crate) const REMAP_YES: i32 = 0;
pub(crate) const REMAP_NONE: i32 = -1;

const UCHAR_MAX: i32 = 255;

pub const MAPTYPE_MAP: i32 = 0;
pub const MAPTYPE_UNMAP: i32 = 1;
pub const MAPTYPE_NOREMAP: i32 = 2;
pub const MAPTYPE_UNMAP_LHS: i32 = 3;

const K_CPO_BSLASH_CONST: i32 = b'B' as i32;

const KEYSET_OPTIDX_KEYMAP_DESC: u64 = 1;
const KEYSET_OPTIDX_KEYMAP_CALLBACK: u64 = 8;

pub struct MapAltLink {
    pub keys: [std::cell::Cell<Option<CString>>; 2],
}

#[repr(C)]
pub struct Mapblock {
    pub m_alt: Option<std::rc::Rc<MapAltLink>>,
    pub m_keys: CString,
    pub m_str: CString,
    pub m_orig_str: CString,
    pub m_luaref: LuaRef,
    pub m_keylen: i32,
    pub m_mode: i32,
    pub m_simplified: i32,
    pub m_noremap: i32,
    pub m_silent: u8,
    pub m_nowait: u8,
    pub m_expr: u8,
    pub m_script_ctx: Sctx,
    pub m_desc: Option<CString>,
    pub m_replace_keycodes: bool,
}

#[derive(Clone)]
pub struct MatchedMap {
    pub m_keys: CString,
    pub m_str: CString,
    pub m_alt_keys: Option<CString>,
    pub m_luaref: LuaRef,
    pub m_keylen: i32,
    pub m_mode: i32,
    pub m_noremap: i32,
    pub m_silent: u8,
    pub m_nowait: u8,
    pub m_expr: u8,
    pub m_replace_keycodes: bool,
}

impl Mapblock {
    #[inline]
    pub fn alt_keys(&self) -> Option<CString> {
        let rc = self.m_alt.as_ref()?;
        let slot = &rc.keys[usize::from(self.m_simplified == 0)];
        let v = slot.take();
        slot.set(v.clone());
        v
    }

    #[inline]
    pub fn to_matched(&self) -> MatchedMap {
        MatchedMap {
            m_keys: self.m_keys.clone(),
            m_str: self.m_str.clone(),
            m_alt_keys: self.alt_keys(),
            m_luaref: self.m_luaref,
            m_keylen: self.m_keylen,
            m_mode: self.m_mode,
            m_noremap: self.m_noremap,
            m_silent: self.m_silent,
            m_nowait: self.m_nowait,
            m_expr: self.m_expr,
            m_replace_keycodes: self.m_replace_keycodes,
        }
    }

    #[inline]
    fn clear_alt_or_luaref(&mut self) {
        if let Some(alt) = self.m_alt.take() {
            alt.keys[0].set(None);
            alt.keys[1].set(None);
            if std::rc::Rc::strong_count(&alt) == 1 {
                nlua_clear_ref(&mut self.m_luaref);
            }
        } else {
            nlua_clear_ref(&mut self.m_luaref);
        }
    }
}

impl Drop for Mapblock {
    fn drop(&mut self) {
        self.clear_alt_or_luaref();
    }
}

#[derive(Clone)]
pub struct Typebuf {
    pub tb_buf: Vec<u8>,
    pub tb_noremap: Vec<u8>,
    pub tb_buflen: i32,
    pub tb_off: i32,
    pub tb_len: i32,
    pub tb_maplen: i32,
    pub tb_silent: i32,
    pub tb_no_abbr_cnt: i32,
    pub tb_change_cnt: i32,
}

impl Typebuf {
    pub const ZERO: Self = Self {
        tb_buf: Vec::new(),
        tb_noremap: Vec::new(),
        tb_buflen: 0,
        tb_off: 0,
        tb_len: 0,
        tb_maplen: 0,
        tb_silent: 0,
        tb_no_abbr_cnt: 0,
        tb_change_cnt: 0,
    };
}

use crate::autocmd::HLF_8;
use crate::ex_getln::{ABBR_OFF, CTRL_RSB_CONST};
use crate::extmark::{K_FALSE, K_NONE};
use crate::insert::CTRL_J_CONST;
use crate::state::{
    MODE_CMDLINE, MODE_INSERT, MODE_LANGMAP, MODE_NORMAL, MODE_OP_PENDING, MODE_SELECT,
    MODE_TERMINAL, MODE_VISUAL,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct LangmapEntry {
    from: i32,
    to: i32,
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
    lhs: [u8; MAXMAPLEN + 1],
    lhs_len: usize,
    alt_lhs: [u8; MAXMAPLEN + 1],
    alt_lhs_len: usize,
    rhs: Option<CString>,
    rhs_len: usize,
    rhs_lua: LuaRef,
    rhs_is_noop: bool,
    orig_rhs: Option<CString>,
    orig_rhs_len: usize,
    desc: Option<CString>,
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
            rhs: None,
            rhs_len: 0,
            rhs_lua: LUA_NOREF,
            rhs_is_noop: false,
            orig_rhs: None,
            orig_rhs_len: 0,
            desc: None,
        }
    }
}

struct MappingCopyState {
    map_luaid: std::cell::Cell<i32>,
    expand_mapmodes: std::cell::Cell<i32>,
    expand_isabbrev: std::cell::Cell<bool>,
    expand_buffer: std::cell::Cell<bool>,
}

impl MappingCopyState {
    const fn new() -> Self {
        Self {
            map_luaid: std::cell::Cell::new(0),
            expand_mapmodes: std::cell::Cell::new(0),
            expand_isabbrev: std::cell::Cell::new(false),
            expand_buffer: std::cell::Cell::new(false),
        }
    }
}

struct MappingTables {
    first_abbr: Vec<Mapblock>,
    maphash: [Vec<Mapblock>; MAX_MAPHASH],
}

impl MappingTables {
    const fn new() -> Self {
        Self {
            first_abbr: Vec::new(),
            maphash: [const { Vec::new() }; MAX_MAPHASH],
        }
    }
}

thread_local! {
    static MAP_STATE: MappingCopyState = const { MappingCopyState::new() };
    static MAP_TABLES: std::cell::RefCell<MappingTables> = const {
        std::cell::RefCell::new(MappingTables::new())
    };
    static LANGMAP_MAPGA: std::cell::RefCell<Vec<LangmapEntry>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

static E_GLOBAL_ABBREVIATION_ALREADY_EXISTS_FOR_STR: &CStr =
    c"E224: Global abbreviation already exists for %s";
static E_GLOBAL_MAPPING_ALREADY_EXISTS_FOR_STR: &CStr =
    c"E225: Global mapping already exists for %s";
static E_ABBREVIATION_ALREADY_EXISTS_FOR_STR: &CStr = c"E226: Abbreviation already exists for %s";
static E_MAPPING_ALREADY_EXISTS_FOR_STR: &CStr = c"E227: Mapping already exists for %s";
static E_ENTRIES_MISSING_IN_MAPSET_DICT_ARGUMENT: &CStr =
    c"E460: Entries missing in mapset() dict argument";
static E_ILLEGAL_MAP_MODE_STRING_STR: &CStr = c"E1276: Illegal map mode string: '%s'";

#[inline]
const fn map_hash(mode: i32, c1: u8) -> usize {
    if (mode & (MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING | MODE_TERMINAL)) != 0 {
        c1 as usize
    } else {
        (c1 ^ 0x80) as usize
    }
}

use crate::keycodes::{k_second, k_third};

/// Run `f` with the global hashed map list for `state` and first character `c`.
#[inline]
pub fn with_maphash_list<R>(state: i32, c: i32, f: impl FnOnce(&[Mapblock]) -> R) -> R {
    let idx = map_hash(state, c as u8);
    MAP_TABLES.with(|cell| f(cell.borrow().maphash[idx].as_slice()))
}

/// Get the buffer-local hashed map list for "state" and first character "c".
pub fn get_buf_maphash_list(state: i32, c: i32) -> &'static [Mapblock] {
    curbuf_mut().b_maphash[map_hash(state, c as u8)].as_slice()
}

#[inline]
fn with_map_tables_mut<R>(
    buf: &mut FileBuf,
    local: bool,
    is_abbrev: bool,
    f: impl FnOnce(&mut [Vec<Mapblock>]) -> R,
) -> R {
    if is_abbrev {
        if local {
            f(std::slice::from_mut(&mut buf.b_first_abbr))
        } else {
            MAP_TABLES.with(|cell| f(std::slice::from_mut(&mut cell.borrow_mut().first_abbr)))
        }
    } else if local {
        f(&mut buf.b_maphash)
    } else {
        MAP_TABLES.with(|cell| f(&mut cell.borrow_mut().maphash))
    }
}

#[inline]
fn with_map_buckets<R>(
    buf: Option<&FileBuf>,
    is_abbrev: bool,
    f: impl FnOnce(&[Vec<Mapblock>]) -> R,
) -> R {
    if is_abbrev {
        if let Some(buf) = buf {
            f(std::slice::from_ref(&buf.b_first_abbr))
        } else {
            MAP_TABLES.with(|cell| f(std::slice::from_ref(&cell.borrow().first_abbr)))
        }
    } else if let Some(buf) = buf {
        f(&buf.b_maphash)
    } else {
        MAP_TABLES.with(|cell| f(&cell.borrow().maphash))
    }
}

/// Finds the Lua mapping whose `m_str` embeds `id`.
fn map_luaid_scan(mappings: &[Vec<Mapblock>], id: i32) -> Option<LuaRef> {
    for bucket in mappings {
        for mp in bucket {
            let s_bytes = mp.m_str.as_bytes();
            if mp.m_luaref != LUA_NOREF
                && crate::charset::parse_i64_prefix(s_bytes.get(3..).unwrap_or(&[])) as i32 == id
            {
                return Some(mp.m_luaref);
            }
        }
    }
    None
}

/// Resolves a Lua-mapping id to its callback.
pub fn map_luaid_get(n: i32) -> LuaRef {
    let mut luaref = MAP_TABLES.with(|cell| {
        let t = cell.borrow();
        map_luaid_scan(&t.maphash, n)
            .or_else(|| map_luaid_scan(std::slice::from_ref(&t.first_abbr), n))
    });
    for fb in for_all_buffers_mut() {
        if luaref.is_none() {
            luaref = map_luaid_scan(&fb.b_maphash, n);
        }
        if luaref.is_none() {
            luaref = map_luaid_scan(std::slice::from_ref(&fb.b_first_abbr), n);
        }
    }
    luaref.unwrap_or(LUA_NOREF)
}

/// Put characters to represent the map mode in a string buffer.
pub fn map_mode_to_chars(mode: i32, buf: &mut [u8]) {
    let mut idx = 0usize;
    if (mode & (MODE_INSERT | MODE_CMDLINE)) == (MODE_INSERT | MODE_CMDLINE) {
        buf[idx] = b'!';
        idx += 1;
    } else if (mode & MODE_INSERT) != 0 {
        buf[idx] = b'i';
        idx += 1;
    } else if (mode & MODE_LANGMAP) != 0 {
        buf[idx] = b'l';
        idx += 1;
    } else if (mode & MODE_CMDLINE) != 0 {
        buf[idx] = b'c';
        idx += 1;
    } else if (mode & (MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING))
        == (MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING)
    {
        buf[idx] = b' ';
        idx += 1;
    } else {
        if (mode & MODE_NORMAL) != 0 {
            buf[idx] = b'n';
            idx += 1;
        }
        if (mode & MODE_OP_PENDING) != 0 {
            buf[idx] = b'o';
            idx += 1;
        }
        if (mode & MODE_TERMINAL) != 0 {
            buf[idx] = b't';
            idx += 1;
        }
        if (mode & (MODE_VISUAL | MODE_SELECT)) == (MODE_VISUAL | MODE_SELECT) {
            buf[idx] = b'v';
            idx += 1;
        } else {
            if (mode & MODE_VISUAL) != 0 {
                buf[idx] = b'x';
                idx += 1;
            }
            if (mode & MODE_SELECT) != 0 {
                buf[idx] = b's';
                idx += 1;
            }
        }
    }
    buf[idx] = 0;
}

fn showmap(mp: &Mapblock, local: bool) {
    if message_filtered(&mp.m_keys)
        && message_filtered(&mp.m_str)
        && mp.m_desc.as_ref().is_none_or(|d| message_filtered(d))
    {
        return;
    }

    if g_st().msg_col > 0 || g_st().msg_silent != 0 {
        msg_putchar(b'\n' as i32);
        if g_st().got_int {
            return;
        }
    }

    let mut mapchars = [0u8; 7];
    map_mode_to_chars(mp.m_mode, &mut mapchars);
    msg_puts(crate::memory::bytes_as_cstr(&mapchars));
    let mut len = crate::memory::bytes_until_nul(&mapchars).len();

    loop {
        len += 1;
        if len > 3 {
            break;
        }
        msg_putchar(b' ' as i32);
    }

    len = msg_outtrans_special(mp.m_keys.as_bytes(), true, 0) as usize;
    loop {
        msg_putchar(b' ' as i32);
        len += 1;
        if len >= 12 {
            break;
        }
    }

    if mp.m_noremap == REMAP_NONE {
        msg_puts_hl(c"*", HLF_8, false);
    } else if mp.m_noremap == REMAP_SCRIPT {
        msg_puts_hl(c"&", HLF_8, false);
    } else {
        msg_putchar(b' ' as i32);
    }

    if local {
        msg_putchar(b'@' as i32);
    } else {
        msg_putchar(b' ' as i32);
    }

    if mp.m_luaref != LUA_NOREF {
        let str_cstr = nlua_funcref_str(mp.m_luaref, true);
        msg_puts_hl(&str_cstr, HLF_8, false);
    } else if mp.m_str.as_bytes().is_empty() {
        msg_puts_hl(c"<Nop>", HLF_8, false);
    } else {
        msg_outtrans_special(mp.m_str.as_bytes(), false, 0);
    }

    if let Some(ref desc) = mp.m_desc {
        msg_puts(c"\n                 ");
        msg_puts(desc);
    }
    if opt_vars().p_verbose > 0 {
        last_set_msg(mp.m_script_ctx);
    }
    msg_clr_eos();
}

fn set_maparg_lhs_rhs(
    orig_lhs: &[u8],
    orig_rhs: &[u8],
    rhs_lua: LuaRef,
    cpo_bytes: &[u8],
    mapargs: &mut MapArguments,
) -> bool {
    mapargs.rhs_lua = rhs_lua;

    let mut did_simplify = false;
    let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
    let Some(replaced) =
        replace_termcodes(orig_lhs, true, 0, flags, Some(&mut did_simplify), cpo_bytes)
    else {
        return false;
    };
    mapargs.lhs_len = replaced.as_bytes().len();
    crate::memory::copy_bytes_to_c_buf(&mut mapargs.lhs, replaced.as_bytes());
    if did_simplify {
        let Some(alt_replaced) = replace_termcodes(
            orig_lhs,
            true,
            0,
            flags | REPTERM_NO_SIMPLIFY,
            None,
            cpo_bytes,
        ) else {
            return false;
        };
        mapargs.alt_lhs_len = alt_replaced.as_bytes().len();
        crate::memory::copy_bytes_to_c_buf(&mut mapargs.alt_lhs, alt_replaced.as_bytes());
    } else {
        mapargs.alt_lhs_len = 0;
    }

    set_maparg_rhs(orig_rhs, rhs_lua, 0, cpo_bytes, mapargs);
    true
}

fn set_maparg_rhs(
    orig_rhs: &[u8],
    rhs_lua: LuaRef,
    sid: Scid,
    cpo_bytes: &[u8],
    mapargs: &mut MapArguments,
) {
    let orig_rhs_len = orig_rhs.len();
    mapargs.rhs_lua = rhs_lua;

    if rhs_lua == LUA_NOREF {
        mapargs.orig_rhs_len = orig_rhs_len;
        let orig_rhs_cstr = CString::new(orig_rhs).unwrap();
        if orig_rhs.eq_ignore_ascii_case(b"<nop>") {
            mapargs.orig_rhs = Some(orig_rhs_cstr);
            mapargs.rhs = Some(CString::default());
            mapargs.rhs_len = 0;
            mapargs.rhs_is_noop = true;
        } else {
            let replaced_cstr =
                replace_termcodes(orig_rhs, false, sid, REPTERM_DO_LT, None, cpo_bytes)
                    .unwrap_or_default();
            mapargs.orig_rhs = Some(orig_rhs_cstr);
            mapargs.rhs_len = replaced_cstr.as_bytes().len();
            mapargs.rhs_is_noop = orig_rhs_len != 0 && mapargs.rhs_len == 0;
            mapargs.rhs = Some(replaced_cstr);
        }
    } else {
        let mut tmp_buf = [0u8; 64];
        mapargs.orig_rhs = Some(CString::default());
        mapargs.orig_rhs_len = 0;
        MAP_STATE.with(|s| s.map_luaid.set(s.map_luaid.get() + (1)));
        mapargs.rhs_len = format_to_buf!(
            &mut tmp_buf,
            c"%c%c%c%d\r",
            K_SPECIAL as i32,
            KS_EXTRA as i32,
            KE_LUA as i32,
            MAP_STATE.with(|s| s.map_luaid.get()),
        ) as usize;
        mapargs.rhs = Some(crate::memory::cstring_until_nul(&tmp_buf));
    }
}

fn str_to_mapargs(strargs: &CStr, is_unmap: bool, mapargs: &mut MapArguments) -> i32 {
    {
        let mut to_parse = crate::charset::skipwhite(strargs);
        *mapargs = MapArguments::new();
        mapargs.rhs_lua = 0;

        loop {
            let bytes = to_parse.to_bytes();
            if bytes.starts_with(b"<buffer>") {
                to_parse =
                    crate::charset::skipwhite(crate::memory::CStrSubslice::advance(to_parse, 8));
                mapargs.buffer = true;
                continue;
            }
            if bytes.starts_with(b"<nowait>") {
                to_parse =
                    crate::charset::skipwhite(crate::memory::CStrSubslice::advance(to_parse, 8));
                mapargs.nowait = true;
                continue;
            }
            if bytes.starts_with(b"<silent>") {
                to_parse =
                    crate::charset::skipwhite(crate::memory::CStrSubslice::advance(to_parse, 8));
                mapargs.silent = true;
                continue;
            }
            if bytes.starts_with(b"<special>") {
                to_parse =
                    crate::charset::skipwhite(crate::memory::CStrSubslice::advance(to_parse, 9));
                continue;
            }
            if bytes.starts_with(b"<script>") {
                to_parse =
                    crate::charset::skipwhite(crate::memory::CStrSubslice::advance(to_parse, 8));
                mapargs.script = true;
                continue;
            }
            if bytes.starts_with(b"<expr>") {
                to_parse =
                    crate::charset::skipwhite(crate::memory::CStrSubslice::advance(to_parse, 6));
                mapargs.expr = true;
                continue;
            }
            if bytes.starts_with(b"<unique>") {
                to_parse =
                    crate::charset::skipwhite(crate::memory::CStrSubslice::advance(to_parse, 8));
                mapargs.unique = true;
                continue;
            }
            break;
        }

        let to_parse_bytes = to_parse.to_bytes();
        let do_backslash =
            !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_BSLASH_CONST);
        let mut orig_lhs_len = 0usize;
        while orig_lhs_len < to_parse_bytes.len()
            && (is_unmap || !ascii_iswhite(to_parse_bytes[orig_lhs_len] as i32))
        {
            if (to_parse_bytes[orig_lhs_len] == CTRL_V_CONST as u8
                || (do_backslash && to_parse_bytes[orig_lhs_len] == b'\\'))
                && orig_lhs_len + 1 < to_parse_bytes.len()
            {
                orig_lhs_len += 1;
            }
            orig_lhs_len += 1;
        }

        let rhs_start = crate::charset::skipwhite_slice(&to_parse_bytes[orig_lhs_len..]);

        if orig_lhs_len >= 256 {
            return 1;
        }

        if !set_maparg_lhs_rhs(
            &to_parse_bytes[..orig_lhs_len],
            rhs_start,
            LUA_NOREF,
            opt_vars().p_cpo.as_bytes(),
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

fn map_add(
    buf: &mut FileBuf,
    local: bool,
    keys: &CStr,
    args: &MapArguments,
    noremap: i32,
    mode: i32,
    is_abbr: bool,
    sid: Scid,
    lnum: Linenr,
    simplified: bool,
    alt: Option<std::rc::Rc<MapAltLink>>,
) {
    {
        if keys.to_bytes().first().copied().unwrap_or(0) == CTRL_C_CONST {
            if local {
                buf.b_mapped_ctrl_c |= mode;
            } else {
                g_st().mapped_ctrl_c |= mode;
            }
        }

        let m_keys = keys.to_owned();
        if let Some(ref alt_rc) = alt {
            alt_rc.keys[usize::from(simplified)].set(Some(m_keys.clone()));
        }
        let m_keylen = m_keys.as_bytes().len() as i32;
        let m_script_ctx = if sid != 0 {
            Sctx {
                sc_sid: sid,
                sc_seq: 0,
                sc_lnum: lnum,
                sc_chan: 0,
            }
        } else {
            let mut ctx = g_st().current_sctx;
            ctx.sc_lnum += sourcing_lnum();
            nlua_set_sctx(&mut ctx);
            ctx
        };
        let mp = Mapblock {
            m_alt: alt,
            m_keys,
            m_str: args.rhs.clone().unwrap_or_default(),
            m_orig_str: args.orig_rhs.clone().unwrap_or_default(),
            m_luaref: args.rhs_lua,
            m_keylen,
            m_mode: mode,
            m_simplified: i32::from(simplified),
            m_noremap: noremap,
            m_silent: u8::from(args.silent),
            m_nowait: u8::from(args.nowait),
            m_expr: u8::from(args.expr),
            m_script_ctx,
            m_desc: args.desc.clone(),
            m_replace_keycodes: args.replace_keycodes,
        };

        if is_abbr {
            if local {
                buf.b_first_abbr.insert(0, mp);
            } else {
                MAP_TABLES.with(|cell| cell.borrow_mut().first_abbr.insert(0, mp));
            }
        } else {
            let n = map_hash(
                mp.m_mode,
                mp.m_keys.to_bytes().first().copied().unwrap_or(0),
            );
            if local {
                (&mut buf.b_maphash)[n].insert(0, mp);
            } else {
                MAP_TABLES.with(|cell| cell.borrow_mut().maphash[n].insert(0, mp));
            }
        }
    }
}

fn buf_do_map(
    mut maptype: i32,
    args: &mut MapArguments,
    mode: i32,
    is_abbrev: bool,
    buf: &mut FileBuf,
) -> i32 {
    let mut retval: i32 = 0;
    let local = args.buffer;
    let mut created_any = false;
    let alt_link = std::rc::Rc::new(MapAltLink {
        keys: [std::cell::Cell::new(None), std::cell::Cell::new(None)],
    });

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

    let has_lhs = args.lhs[0] != 0;
    let has_rhs = args.rhs_lua != LUA_NOREF
        || args.rhs.as_ref().is_some_and(|s| !s.as_bytes().is_empty())
        || args.rhs_is_noop;
    let do_print = !has_lhs || (maptype != MAPTYPE_UNMAP && !has_rhs);
    if do_print {
        msg_ext_set_kind(c"list_cmd");
    }

    if maptype == MAPTYPE_UNMAP && !has_lhs {
        return 1;
    }

    let did_simplify = args.alt_lhs_len != 0;

    'outer: {
        for keyround in 1..=2 {
            let mut did_it = false;
            let mut did_local = false;
            let keyround1_simplified = keyround == 1 && did_simplify;
            let mut len = args.lhs_len as i32;
            let mut lhs_arr = args.lhs;

            if keyround == 2 {
                if !did_simplify {
                    break;
                }
                lhs_arr = args.alt_lhs;
                len = args.alt_lhs_len as i32;
            } else if did_simplify && do_print {
                lhs_arr = args.alt_lhs;
                len = args.alt_lhs_len as i32;
            }
            let lhs_cstr = crate::memory::bytes_as_cstr(&lhs_arr);
            let lhs_bytes = lhs_cstr.to_bytes();

            if has_lhs {
                if (len as usize) > MAXMAPLEN {
                    retval = 1;
                    break 'outer;
                }

                let lhs_full_slice = &lhs_bytes[..(len as usize).min(lhs_bytes.len())];
                if is_abbrev && maptype != MAPTYPE_UNMAP {
                    let mut same: i32 = -1;
                    let mut keys_unescaped = [0u8; MAXMAPLEN + 1];
                    keys_unescaped[..lhs_full_slice.len()].copy_from_slice(lhs_full_slice);
                    let keys_unescaped_len = vim_unescape_ks(&mut keys_unescaped);
                    let unesc_slice = &keys_unescaped[..keys_unescaped_len];
                    let mut p_off = 0usize;

                    let first = vim_iswordp(unesc_slice);
                    let mut last = first;
                    p_off += (utfc_ptr2len(unesc_slice) as usize).min(unesc_slice.len());
                    let mut n: i32 = 1;
                    while p_off < keys_unescaped_len {
                        n += 1;
                        let rem = &unesc_slice[p_off..];
                        last = vim_iswordp(rem);
                        if same == -1 && last != first {
                            same = n - 1;
                        }
                        p_off += (utfc_ptr2len(rem) as usize).min(rem.len());
                    }
                    if last && n > 2 && same >= 0 && same < n - 1 {
                        retval = 1;
                        break 'outer;
                    }

                    for &b in lhs_full_slice {
                        if ascii_iswhite(i32::from(b)) {
                            retval = 1;
                            break 'outer;
                        }
                    }
                }
            }

            if has_lhs && has_rhs && is_abbrev {
                g_st().no_abbr = false;
            }

            if do_print {
                msg_start();
            }

            if args.unique && local && has_lhs && has_rhs && maptype != MAPTYPE_UNMAP {
                let lhs_slice = &lhs_bytes[..(len as usize).min(lhs_bytes.len())];
                let dup_found = with_map_buckets(None, is_abbrev, |buckets| {
                    for bucket in buckets {
                        if g_st().got_int {
                            break;
                        }
                        for mp in bucket {
                            if g_st().got_int {
                                break;
                            }
                            if (mp.m_mode & mode) != 0 && mp.m_keylen == len {
                                let mp_slice = mp.m_keys.as_bytes();
                                if mp_slice == lhs_slice {
                                    return true;
                                }
                            }
                        }
                    }
                    false
                });
                if dup_found {
                    retval = 6;
                    break 'outer;
                }
            }

            if !local && !has_rhs && maptype != MAPTYPE_UNMAP {
                with_map_buckets(Some(&*buf), is_abbrev, |buckets| {
                    for bucket in buckets {
                        if g_st().got_int {
                            break;
                        }
                        for mp in bucket {
                            if g_st().got_int {
                                break;
                            }
                            if mp.m_simplified == 0 && (mp.m_mode & mode) != 0 {
                                if !has_lhs {
                                    showmap(mp, true);
                                    did_local = true;
                                } else {
                                    let n = mp.m_keylen;
                                    let cmp_len = n.min(len) as usize;
                                    let mp_slice = &mp.m_keys.as_bytes()[..cmp_len];
                                    let lhs_slice = &lhs_bytes[..cmp_len.min(lhs_bytes.len())];
                                    if mp_slice == lhs_slice {
                                        showmap(mp, true);
                                        did_local = true;
                                    }
                                }
                            }
                        }
                    }
                });
            }

            let num_rounds = if maptype == MAPTYPE_UNMAP && !unmap_lhs_only {
                2
            } else {
                1
            };
            let unique_conflict = with_map_tables_mut(buf, local, is_abbrev, |tables| {
                for round in 0..num_rounds {
                    if did_it || g_st().got_int {
                        break;
                    }
                    let (hash_start, hash_end) = if (round == 0 && has_lhs) || is_abbrev {
                        let start = if is_abbrev {
                            0
                        } else {
                            map_hash(mode, lhs_arr[0])
                        };
                        (start, start + 1)
                    } else {
                        (0, 256)
                    };
                    for hash in hash_start..hash_end {
                        if g_st().got_int {
                            break;
                        }
                        let mut idx: usize = 0;
                        while idx < tables[hash].len() && !g_st().got_int {
                            let mp: &mut Mapblock = &mut tables[hash][idx];
                            if (mp.m_mode & mode) == 0 {
                                idx += 1;
                                continue;
                            }
                            if !has_lhs {
                                if mp.m_simplified == 0 {
                                    showmap(mp, local);
                                    did_it = true;
                                }
                            } else {
                                let (n, p_bytes) = if round != 0 {
                                    (mp.m_str.as_bytes().len() as i32, mp.m_str.as_bytes())
                                } else {
                                    (mp.m_keylen, mp.m_keys.as_bytes())
                                };
                                let cmp_len = n.min(len) as usize;
                                let p_slice = &p_bytes[..cmp_len.min(p_bytes.len())];
                                let lhs_slice = &lhs_bytes[..cmp_len.min(lhs_bytes.len())];
                                if p_slice == lhs_slice {
                                    if maptype == MAPTYPE_UNMAP {
                                        if n != len
                                            && (!is_abbrev
                                                || round != 0
                                                || n > len
                                                || !crate::charset::skipwhite_slice(
                                                    &lhs_bytes[(n as usize).min(lhs_bytes.len())..],
                                                )
                                                .is_empty())
                                        {
                                            idx += 1;
                                            continue;
                                        }
                                        if keyround1_simplified && mp.m_simplified == 0 {
                                            break;
                                        }
                                        mp.m_mode &= !mode;
                                        did_it = true;
                                    } else if !has_rhs {
                                        if mp.m_simplified == 0 {
                                            showmap(mp, local);
                                            did_it = true;
                                        }
                                    } else if n != len {
                                        idx += 1;
                                        continue;
                                    } else if keyround1_simplified && mp.m_simplified == 0 {
                                        did_it = true;
                                        break;
                                    } else if args.unique {
                                        return true;
                                    } else {
                                        mp.m_mode &= !mode;
                                        if mp.m_mode == 0 && !did_it {
                                            mp.clear_alt_or_luaref();
                                            mp.m_str = args.rhs.clone().unwrap_or_default();
                                            mp.m_orig_str =
                                                args.orig_rhs.clone().unwrap_or_default();
                                            mp.m_luaref = args.rhs_lua;
                                            mp.m_noremap = noremap;
                                            mp.m_nowait = u8::from(args.nowait);
                                            mp.m_silent = u8::from(args.silent);
                                            mp.m_mode = mode;
                                            mp.m_simplified = i32::from(keyround1_simplified);
                                            mp.m_expr = u8::from(args.expr);
                                            mp.m_replace_keycodes = args.replace_keycodes;
                                            mp.m_script_ctx = g_st().current_sctx;
                                            mp.m_script_ctx.sc_lnum += sourcing_lnum();
                                            nlua_set_sctx(&mut mp.m_script_ctx);
                                            mp.m_desc = args.desc.clone();
                                            alt_link.keys[usize::from(keyround1_simplified)]
                                                .set(Some(mp.m_keys.clone()));
                                            mp.m_alt = Some(alt_link.clone());
                                            created_any = true;
                                            did_it = true;
                                        }
                                    }
                                    if mp.m_mode == 0 {
                                        tables[hash].remove(idx);
                                        continue;
                                    }

                                    let new_hash = map_hash(
                                        mp.m_mode,
                                        mp.m_keys.to_bytes().first().copied().unwrap_or(0),
                                    );
                                    if !is_abbrev && new_hash != hash {
                                        let moved = tables[hash].remove(idx);
                                        tables[new_hash].insert(0, moved);
                                        continue;
                                    }
                                }
                            }
                            idx += 1;
                        }
                    }
                }
                false
            });
            if unique_conflict {
                retval = 5;
                break 'outer;
            }

            if maptype == MAPTYPE_UNMAP {
                if !did_it {
                    if !keyround1_simplified {
                        retval = 2;
                    }
                } else if lhs_arr[0] == CTRL_C_CONST {
                    if local {
                        buf.b_mapped_ctrl_c &= !mode;
                    } else {
                        g_st().mapped_ctrl_c &= !mode;
                    }
                }
                continue;
            }

            if !has_lhs || !has_rhs {
                if !did_it && !did_local {
                    if is_abbrev {
                        msg(gettext(c"No abbreviation found"), 0);
                    } else {
                        msg(gettext(c"No mapping found"), 0);
                    }
                }
                break 'outer;
            }

            if did_it {
                continue;
            }

            map_add(
                buf,
                local,
                lhs_cstr,
                args,
                noremap,
                mode,
                is_abbrev,
                0,
                0,
                keyround1_simplified,
                Some(alt_link.clone()),
            );
            created_any = true;
        }
    }

    if std::rc::Rc::strong_count(&alt_link) < 3 {
        alt_link.keys[0].set(None);
        alt_link.keys[1].set(None);
    }

    if created_any {
        args.rhs = None;
        args.orig_rhs = None;
        args.rhs_lua = LUA_NOREF;
        args.desc = None;
    }
    retval
}

/// Set or remove a mapping or an abbreviation in the current buffer, OR
/// display (matching) mappings/abbreviations.
pub fn do_map(maptype: i32, arg: &CStr, mode: i32, is_abbrev: bool) -> i32 {
    let mut parsed_args = MapArguments::new();
    let mut result = str_to_mapargs(arg, maptype == MAPTYPE_UNMAP, &mut parsed_args);
    match result {
        0 => {
            result = buf_do_map(maptype, &mut parsed_args, mode, is_abbrev, curbuf_mut());
        }
        1 => {}
        _ => {
            debug_assert!(false, "Unknown return code from str_to_mapargs!");
            result = -1;
        }
    }
    result
}

/// Get the mapping mode from the command name.
fn get_map_mode(cmdp: &mut &[u8], forceit: bool) -> i32 {
    let modec = cmdp.first().copied().unwrap_or(0);
    let mut adv = 1usize.min(cmdp.len());
    let mode = if modec == b'i' {
        MODE_INSERT
    } else if modec == b'l' {
        MODE_LANGMAP
    } else if modec == b'c' {
        MODE_CMDLINE
    } else if modec == b'n' && cmdp.get(1).copied().unwrap_or(0) != b'o' {
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
        adv = 0;
        if forceit {
            MODE_INSERT | MODE_CMDLINE
        } else {
            MODE_VISUAL | MODE_SELECT | MODE_NORMAL | MODE_OP_PENDING
        }
    };

    *cmdp = &cmdp[adv..];
    mode
}

fn do_mapclear(cmdp: &CStr, arg: &CStr, forceit: bool, abbr: bool) {
    let mut cmd_bytes = cmdp.to_bytes();
    let arg_bytes = arg.to_bytes();
    let local = arg_bytes == b"<buffer>";
    if !local && !arg_bytes.is_empty() {
        emsg(gettext(E_INVARG));
        return;
    }

    let mode = get_map_mode(&mut cmd_bytes, forceit);
    map_clear_mode(curbuf_mut(), mode, local, abbr);
}

/// Clear all mappings in "mode".
pub fn map_clear_mode(buf: &mut FileBuf, mode: i32, local: bool, abbr: bool) {
    with_map_tables_mut(buf, local, abbr, |tables| {
        for hash in 0..tables.len() {
            let mut idx: usize = 0;
            while idx < tables[hash].len() {
                let mp: &mut Mapblock = &mut tables[hash][idx];
                if (mp.m_mode & mode) != 0 {
                    mp.m_mode &= !mode;
                    if mp.m_mode == 0 {
                        tables[hash].remove(idx);
                        continue;
                    }
                    let new_hash = map_hash(
                        mp.m_mode,
                        mp.m_keys.to_bytes().first().copied().unwrap_or(0),
                    );
                    if !abbr && new_hash != hash {
                        let moved = tables[hash].remove(idx);
                        tables[new_hash].insert(0, moved);
                        continue;
                    }
                }
                idx += 1;
            }
        }
    });
}

/// Check if a map exists that has given string in the rhs.
pub fn map_to_exists(str_ptr: &CStr, modechars: &CStr, abbr: bool) -> bool {
    let mut mode: i32 = 0;
    let rhs = replace_termcodes(
        str_ptr.to_bytes(),
        false,
        0,
        REPTERM_DO_LT,
        None,
        opt_vars().p_cpo.as_bytes(),
    )
    .unwrap_or_default();

    let mode_bytes = modechars.to_bytes();
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

    map_to_exists_mode(rhs.as_c_str(), mode, abbr)
}

/// Check if a map exists that has given string in the rhs.
pub fn map_to_exists_mode(rhs: &CStr, mode: i32, abbr: bool) -> bool {
    let rhs_b = rhs.to_bytes();
    for exp_buffer in [false, true] {
        let b = if exp_buffer {
            g_st().curbuf.opt()
        } else {
            None
        };
        let found = with_map_buckets(b, abbr, |buckets| {
            for bucket in buckets {
                for mp in bucket {
                    if (mp.m_mode & mode) != 0
                        && (rhs_b.is_empty()
                            || mp.m_str.as_bytes().windows(rhs_b.len()).any(|w| w == rhs_b))
                    {
                        return true;
                    }
                }
            }
            false
        });
        if found {
            return true;
        }
    }

    false
}

fn translate_mapping(bytes: &[u8], cpo_val: &CStr) -> CString {
    let mut idx = 0usize;
    let mut buf: Vec<u8> = Vec::with_capacity(40);

    let cpo_bslash = crate::strings::vim_haschr(cpo_val, K_CPO_BSLASH_CONST);

    while idx < bytes.len() {
        let mut c = i32::from(bytes[idx]);
        if c == K_SPECIAL as i32 && idx + 2 < bytes.len() {
            let mut modifiers: i32 = 0;
            if bytes[idx + 1] == KS_MODIFIER {
                modifiers = i32::from(bytes[idx + 2]);
                idx += 3;
                c = bytes.get(idx).copied().map_or(0, i32::from);
            }

            if c == K_SPECIAL as i32 && idx + 2 < bytes.len() {
                c = to_special(i32::from(bytes[idx + 1]), i32::from(bytes[idx + 2]));
                if c == K_ZERO {
                    c = NUL as i32;
                }
                idx += 2;
            }
            if is_special(c) || modifiers != 0 {
                let s = get_special_key_name(c, modifiers);
                buf.extend_from_slice(s.to_bytes());
                idx += 1;
                continue;
            }
        }

        if c == b' ' as i32
            || c == b'\t' as i32
            || c == CTRL_J_CONST
            || c == CTRL_V_CONST
            || c == b'<' as i32
            || (c == b'\\' as i32 && !cpo_bslash)
        {
            buf.push(if cpo_bslash {
                CTRL_V_CONST as u8
            } else {
                b'\\'
            });
        }

        if c != 0 {
            buf.push(c as u8);
        }
        idx += 1;
    }
    crate::memory::bytes_to_cstring(&buf)
}

/// Work out what to complete when doing command line completion of mapping
/// or abbreviation names.
pub fn set_context_in_map_cmd(
    xp: &mut ExpandCtx,
    cmd: &CStr,
    mut arg: &CStr,
    forceit: bool,
    isabbrev: bool,
    isunmap: bool,
    cmdidx: crate::cmdexpand::Cmdidx,
) -> Option<&'static CStr> {
    let mut cmd_bytes = cmd.to_bytes();
    if forceit
        && cmdidx != crate::cmdexpand::Cmdidx::CmdMap
        && cmdidx != crate::cmdexpand::Cmdidx::CmdUnmap
    {
        xp.xp_context = EXPAND_NOTHING;
    } else {
        if isunmap {
            MAP_STATE.with(|s| {
                s.expand_mapmodes
                    .set(get_map_mode(&mut cmd_bytes, forceit || isabbrev))
            });
        } else {
            MAP_STATE.with(|s| s.expand_mapmodes.set(MODE_INSERT | MODE_CMDLINE));
            if !isabbrev {
                MAP_STATE.with(|s| {
                    s.expand_mapmodes.set(
                        s.expand_mapmodes.get()
                            | (MODE_VISUAL | MODE_SELECT | MODE_NORMAL | MODE_OP_PENDING),
                    )
                });
            }
        }
        MAP_STATE.with(|s| s.expand_isabbrev.set(isabbrev));
        xp.xp_context = EXPAND_MAPPINGS;
        MAP_STATE.with(|s| s.expand_buffer.set(false));
        loop {
            let bytes = arg.to_bytes();
            if bytes.starts_with(b"<buffer>") {
                MAP_STATE.with(|s| s.expand_buffer.set(true));
                arg = &arg[8 + crate::charset::skipwhite_len(&bytes[8..])..];
                continue;
            }
            if bytes.starts_with(b"<unique>") {
                arg = &arg[8 + crate::charset::skipwhite_len(&bytes[8..])..];
                continue;
            }
            if bytes.starts_with(b"<nowait>") {
                arg = &arg[8 + crate::charset::skipwhite_len(&bytes[8..])..];
                continue;
            }
            if bytes.starts_with(b"<silent>") {
                arg = &arg[8 + crate::charset::skipwhite_len(&bytes[8..])..];
                continue;
            }
            if bytes.starts_with(b"<special>") {
                arg = &arg[9 + crate::charset::skipwhite_len(&bytes[9..])..];
                continue;
            }
            if bytes.starts_with(b"<script>") {
                arg = &arg[8 + crate::charset::skipwhite_len(&bytes[8..])..];
                continue;
            }
            if bytes.starts_with(b"<expr>") {
                arg = &arg[6 + crate::charset::skipwhite_len(&bytes[6..])..];
                continue;
            }
            break;
        }
        xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(arg);
    }

    None
}

/// Find all mapping/abbreviation names that match regexp "regmatch".
pub fn expand_mappings(pat: &CStr, regmatch: &mut Regmatch, matches: &mut Vec<CString>) -> bool {
    let fuzzy = cmdline_fuzzy_complete(pat);

    matches.clear();

    let mut str_matches: Vec<CString> = Vec::new();
    let mut fuz_matches: Vec<FuzmatchStr> = Vec::new();

    for i in 0..7 {
        let p_cstr: &CStr = match i {
            0 => c"<silent>",
            1 => c"<unique>",
            2 => c"<script>",
            3 => c"<expr>",
            4 if !MAP_STATE.with(|s| s.expand_buffer.get()) => c"<buffer>",
            5 => c"<nowait>",
            6 => c"<special>",
            _ => continue,
        };
        let mut score: i32 = 0;
        let match_found = if !fuzzy {
            vim_regexec(regmatch, p_cstr, 0)
        } else {
            score = fuzzy_match_str(Some(p_cstr.to_bytes()), Some(pat.to_bytes()));
            score != FUZZY_SCORE_NONE
        };

        if !match_found {
            continue;
        }

        if fuzzy {
            let idx = fuz_matches.len() as i32;
            fuz_matches.push(FuzmatchStr {
                idx,
                str_cstr: p_cstr.to_owned(),
                score,
            });
        } else {
            str_matches.push(p_cstr.to_owned());
        }
    }

    let exp_buf = if !MAP_STATE.with(|s| s.expand_isabbrev.get())
        && MAP_STATE.with(|s| s.expand_buffer.get())
    {
        g_st().curbuf.opt()
    } else {
        None
    };
    let is_abbrev = MAP_STATE.with(|s| s.expand_isabbrev.get());
    let mapmodes = MAP_STATE.with(|s| s.expand_mapmodes.get());
    with_map_buckets(exp_buf, is_abbrev, |buckets| {
        for bucket in buckets {
            for mp in bucket {
                if mp.m_simplified != 0 || (mp.m_mode & mapmodes) == 0 {
                    continue;
                }

                let p_cstr = translate_mapping(mp.m_keys.as_bytes(), opt_vars().p_cpo.as_cstr());

                let mut score: i32 = 0;
                let match_found = if !fuzzy {
                    vim_regexec(regmatch, &p_cstr, 0)
                } else {
                    score = fuzzy_match_str(Some(p_cstr.to_bytes()), Some(pat.to_bytes()));
                    score != FUZZY_SCORE_NONE
                };

                if !match_found {
                    continue;
                }

                if fuzzy {
                    let idx = fuz_matches.len() as i32;
                    fuz_matches.push(FuzmatchStr {
                        idx,
                        str_cstr: p_cstr,
                        score,
                    });
                } else {
                    str_matches.push(p_cstr);
                }
            }
        }
    });

    if if fuzzy {
        fuz_matches.is_empty()
    } else {
        str_matches.is_empty()
    } {
        return false;
    }

    if !fuzzy {
        *matches = str_matches;
    } else {
        fuzzymatches_slice_to_strmatches(&mut fuz_matches, matches, false);
    }

    if matches.len() > 1 {
        if !fuzzy {
            sort_strings(matches);
        }
        matches.dedup();
    }

    !matches.is_empty()
}

/// Check for an abbreviation.
pub fn check_abbr(mut c: i32, line_cstr: &CStr, col: i32, mincol: i32) -> bool {
    let line_bytes = line_cstr.to_bytes();
    let mut tb = [0u8; MB_MAXBYTES + 4];

    if g_st().typebuf.tb_no_abbr_cnt != 0 {
        return false;
    }

    if noremap_keys() && c != CTRL_RSB_CONST {
        return false;
    }

    if col == 0 {
        return false;
    }

    let mut clen: i32;
    let mut scol: i32;
    {
        let ptr_bytes = line_cstr.to_bytes();
        let mut is_id = true;
        let col_u = col as usize;
        let mut p_off = if col_u > 0 {
            col_u - 1 - crate::mbyte::utf_head_off_slice(ptr_bytes, col_u - 1) as usize
        } else {
            0
        };
        let vim_abbr = if !vim_iswordp(ptr_bytes.get(p_off..).unwrap_or(&[])) {
            true
        } else {
            if p_off > 0 {
                let prev_off =
                    p_off - 1 - crate::mbyte::utf_head_off_slice(ptr_bytes, p_off - 1) as usize;
                is_id = vim_iswordp(ptr_bytes.get(prev_off..).unwrap_or(&[]));
            }
            false
        };
        clen = 1;
        let min_off = mincol.max(0) as usize;
        while p_off > min_off {
            p_off = p_off - 1 - crate::mbyte::utf_head_off_slice(ptr_bytes, p_off - 1) as usize;
            let p_bytes = ptr_bytes.get(p_off..).unwrap_or(&[]);
            if ascii_isspace(p_bytes.first().copied().unwrap_or(0) as i32)
                || (!vim_abbr && is_id != vim_iswordp(p_bytes))
            {
                p_off += utfc_ptr2len(p_bytes) as usize;
                break;
            }
            clen += 1;
        }
        scol = p_off as i32;
    }

    if scol < mincol {
        scol = mincol;
    }
    if scol < col {
        let scol_u = (scol.max(0) as usize).min(line_bytes.len());
        let mut len = col - scol;
        let mp_opt: Option<MatchedMap> = MAP_TABLES.with(|cell| {
            let tables = cell.borrow();
            let first_abbr_slice = tables.first_abbr.as_slice();
            for cand in curbuf_mut().b_first_abbr.iter().chain(first_abbr_slice) {
                let mut qlen = cand.m_keylen;
                let mut q_vec: Vec<u8>;
                let mut q_bytes = cand.m_keys.as_bytes_with_nul();

                if cand.m_keys.as_bytes().contains(&K_SPECIAL) {
                    q_vec = cand.m_keys.as_bytes_with_nul().to_vec();
                    qlen = vim_unescape_ks(&mut q_vec) as i32;
                    q_bytes = &q_vec;
                }
                let len_u = len as usize;
                let abbr_slice = &line_bytes[scol_u..(scol_u + len_u).min(line_bytes.len())];
                let match_found = (cand.m_mode & g_st().state_global) != 0
                    && qlen == len
                    && &q_bytes[..len_u.min(q_bytes.len())] == abbr_slice;
                if match_found {
                    return Some(cand.to_matched());
                }
            }
            None
        });
        if let Some(ref mp) = mp_opt {
            let mut j: usize = 0;
            if c != CTRL_RSB_CONST {
                if is_special(c) || c == K_SPECIAL as i32 {
                    tb[j] = K_SPECIAL;
                    j += 1;
                    tb[j] = k_second(c);
                    j += 1;
                    tb[j] = k_third(c);
                    j += 1;
                } else {
                    if c < ABBR_OFF && (c < b' ' as i32 || c > b'~' as i32) {
                        tb[j] = CTRL_V_CONST as u8;
                        j += 1;
                    }
                    if c >= ABBR_OFF {
                        c -= ABBR_OFF;
                    }
                    let mut newlen = utf_char2bytes(c, &mut tb[j..]) as usize;
                    tb[j + newlen] = 0;
                    let escaped = vim_strsave_escape_ks(&tb[j..j + newlen]);
                    let escaped_bytes = escaped.to_bytes();
                    newlen = escaped_bytes.len();
                    tb[j..j + newlen].copy_from_slice(escaped_bytes);
                    j += newlen;
                }
                tb[j] = 0;
                ins_typebuf(
                    CStr::from_bytes_until_nul(&tb).unwrap(),
                    1,
                    0,
                    true,
                    mp.m_silent != 0,
                );
            }

            let noremap = mp.m_noremap;
            let silent = mp.m_silent != 0;
            let expr = mp.m_expr != 0;
            let m_str_clone = if !expr { Some(mp.m_str.clone()) } else { None };

            let owned_s = if expr {
                eval_map_expr(mp, c)
            } else {
                m_str_clone
            };
            if let Some(ref cs) = owned_s {
                ins_typebuf(cs, noremap, 0, true, silent);
                g_st().typebuf.tb_no_abbr_cnt += (cs.to_bytes().len() + j + 1) as i32;
            }

            tb[0] = CTRL_H_CONST;
            tb[1] = 0;
            len = clen;
            while len > 0 {
                len -= 1;
                ins_typebuf(CStr::from_bytes_until_nul(&tb).unwrap(), 1, 0, true, silent);
            }
            return true;
        }
    }
    false
}

/// Evaluate the RHS of a mapping or abbreviations and take care of escaping
/// special characters.
pub fn eval_map_expr(mp: &MatchedMap, c: i32) -> Option<CString> {
    let mut p: Option<CString> = None;
    let mut expr_bytes: Vec<u8> = Vec::new();
    let luaref = mp.m_luaref;
    let replace_keycodes = mp.m_replace_keycodes;

    if luaref == LUA_NOREF {
        expr_bytes.extend_from_slice(mp.m_str.to_bytes_with_nul());
        vim_unescape_ks(&mut expr_bytes);
    }

    g_st().expr_map_lock += 1;
    set_vim_var_char(c);
    let save_cursor = curwin_mut().w_cursor;
    let save_msg_col = g_st().msg_col;
    let save_msg_row = g_st().msg_row;
    if luaref != LUA_NOREF {
        let mut err = Error::new();
        let args = Array::new();
        let ret = nlua_call_ref(luaref, None, args, K_RET_OBJECT, None, Some(&mut err));
        if ret.obj_type == ObjectType::ObjectTypeString {
            p = Some(string_to_cstr(ret.data.string()));
        }
        api_free_object(ret);
        if err.err_type != ErrorType::ErrorTypeNone {
            semsg_multiline!(c"emsg", c"E5108: %s", err.msg_cstr());
            api_clear_error(&mut err);
        }
    } else {
        p = eval_to_string(crate::memory::bytes_as_cstr(&expr_bytes), false, false);
    }
    g_st().expr_map_lock -= 1;
    curwin_mut().w_cursor = save_cursor;
    g_st().msg_col = save_msg_col;
    g_st().msg_row = save_msg_row;

    let p = p?;

    if replace_keycodes {
        replace_termcodes(
            p.to_bytes(),
            false,
            0,
            REPTERM_DO_LT,
            None,
            opt_vars().p_cpo.as_bytes(),
        )
    } else {
        Some(vim_strsave_escape_ks(p.to_bytes()))
    }
}

/// Write map commands for the current mappings to an .exrc file.
pub fn makemap(fd: &CFile, buf: Option<&FileBuf>) -> bool {
    let mut did_cpo = false;

    for abbr in 0..2 {
        let ok = with_map_buckets(buf, abbr != 0, |buckets| {
            for bucket in buckets {
                for mp in bucket {
                    if mp.m_simplified != 0
                        || mp.m_noremap == REMAP_SCRIPT
                        || mp.m_luaref != LUA_NOREF
                    {
                        continue;
                    }

                    if mp
                        .m_str
                        .as_bytes()
                        .windows(3)
                        .any(|w| w == [K_SPECIAL, KS_EXTRA, KE_SNR])
                    {
                        continue;
                    }

                    let mut c1: u8 = 0;
                    let mut c2: u8 = 0;
                    let mut c3: u8 = 0;
                    let mut cmd: &CStr = if abbr != 0 { c"abbr" } else { c"map" };

                    const MODE_NVSO: i32 =
                        MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING;
                    const MODE_NV: i32 = MODE_NORMAL | MODE_VISUAL;
                    const MODE_NS: i32 = MODE_NORMAL | MODE_SELECT;
                    const MODE_NO: i32 = MODE_NORMAL | MODE_OP_PENDING;
                    const MODE_VS: i32 = MODE_VISUAL | MODE_SELECT;
                    const MODE_VO: i32 = MODE_VISUAL | MODE_OP_PENDING;
                    const MODE_SO: i32 = MODE_SELECT | MODE_OP_PENDING;
                    const MODE_NVS: i32 = MODE_NORMAL | MODE_VISUAL | MODE_SELECT;
                    const MODE_NVO: i32 = MODE_NORMAL | MODE_VISUAL | MODE_OP_PENDING;
                    const MODE_NSO: i32 = MODE_NORMAL | MODE_SELECT | MODE_OP_PENDING;
                    const MODE_VSO: i32 = MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING;
                    const MODE_CI: i32 = MODE_CMDLINE | MODE_INSERT;

                    match mp.m_mode {
                        MODE_NVSO => {}
                        MODE_NORMAL => c1 = b'n',
                        MODE_VISUAL => c1 = b'x',
                        MODE_SELECT => c1 = b's',
                        MODE_OP_PENDING => c1 = b'o',
                        MODE_NV => {
                            c1 = b'n';
                            c2 = b'x';
                        }
                        MODE_NS => {
                            c1 = b'n';
                            c2 = b's';
                        }
                        MODE_NO => {
                            c1 = b'n';
                            c2 = b'o';
                        }
                        MODE_VS => c1 = b'v',
                        MODE_VO => {
                            c1 = b'x';
                            c2 = b'o';
                        }
                        MODE_SO => {
                            c1 = b's';
                            c2 = b'o';
                        }
                        MODE_NVS => {
                            c1 = b'n';
                            c2 = b'v';
                        }
                        MODE_NVO => {
                            c1 = b'n';
                            c2 = b'x';
                            c3 = b'o';
                        }
                        MODE_NSO => {
                            c1 = b'n';
                            c2 = b's';
                            c3 = b'o';
                        }
                        MODE_VSO => {
                            c1 = b'v';
                            c2 = b'o';
                        }
                        MODE_CI => {
                            if abbr == 0 {
                                cmd = c"map!";
                            }
                        }
                        MODE_CMDLINE => c1 = b'c',
                        MODE_INSERT => c1 = b'i',
                        MODE_LANGMAP => c1 = b'l',
                        MODE_TERMINAL => c1 = b't',
                        _ => {
                            iemsg(gettext(c"E228: makemap: Illegal mode"));
                            return false;
                        }
                    }

                    loop {
                        if !did_cpo {
                            if mp.m_str.as_bytes().is_empty() {
                                did_cpo = true;
                            } else {
                                let specials = [K_SPECIAL, NL as u8];
                                if mp.m_str.as_bytes().iter().any(|b| specials.contains(b))
                                    || mp.m_keys.as_bytes().iter().any(|b| specials.contains(b))
                                {
                                    did_cpo = true;
                                }
                            }
                            if did_cpo
                                && (fd.puts(c"let s:cpo_save=&cpo") < 0
                                    || !put_eol(fd)
                                    || fd.puts(c"set cpo&vim") < 0
                                    || !put_eol(fd))
                            {
                                return false;
                            }
                        }
                        if c1 != 0 && fd.putc(c1 as i32) < 0 {
                            return false;
                        }
                        if mp.m_noremap != REMAP_YES && fd.puts(c"nore") < 0 {
                            return false;
                        }
                        if fd.puts(cmd) < 0 {
                            return false;
                        }
                        if buf.is_some() && fd.puts(c" <buffer>") < 0 {
                            return false;
                        }
                        if mp.m_nowait != 0 && fd.puts(c" <nowait>") < 0 {
                            return false;
                        }
                        if mp.m_silent != 0 && fd.puts(c" <silent>") < 0 {
                            return false;
                        }
                        if mp.m_expr != 0 && fd.puts(c" <expr>") < 0 {
                            return false;
                        }

                        if fd.putc(b' ' as i32) < 0
                            || !put_escstr(fd, mp.m_keys.as_c_str(), 0)
                            || fd.putc(b' ' as i32) < 0
                            || !put_escstr(fd, mp.m_str.as_c_str(), 1)
                            || !put_eol(fd)
                        {
                            return false;
                        }
                        c1 = c2;
                        c2 = c3;
                        c3 = 0;
                        if c1 == 0 {
                            break;
                        }
                    }
                }
            }
            true
        });
        if !ok {
            return false;
        }
    }
    if did_cpo
        && (fd.puts(c"let &cpo=s:cpo_save") < 0
            || !put_eol(fd)
            || fd.puts(c"unlet s:cpo_save") < 0
            || !put_eol(fd))
    {
        return false;
    }
    true
}

/// Write escape string to file.
pub fn put_escstr(fd: &CFile, strstart: &CStr, what: i32) -> bool {
    let full_bytes = strstart.to_bytes();
    let mut rem = full_bytes;

    if rem.is_empty() && what == 1 {
        if fd.puts(c"<Nop>") < 0 {
            return false;
        }
        return true;
    }

    while !rem.is_empty() {
        if let Some(p) = mb_unescape(&mut rem) {
            for &p_byte in p.to_bytes() {
                if fd.putc(p_byte as i32) < 0 {
                    return false;
                }
            }
            continue;
        }

        let is_start = rem.len() == full_bytes.len();
        let mut c = i32::from(rem[0]);
        if c == K_SPECIAL as i32 && what != 2 {
            let mut modifiers: i32 = 0;
            let mut consumed_unescaped = false;
            if rem.get(1) == Some(&KS_MODIFIER) {
                modifiers = i32::from(rem.get(2).copied().unwrap_or(0));
                rem = &rem[3.min(rem.len())..];

                if let Some(p) = mb_unescape(&mut rem) {
                    c = utf_slice2char(p.to_bytes());
                    consumed_unescaped = true;
                } else {
                    c = i32::from(rem.first().copied().unwrap_or(0));
                }
            }
            if c == K_SPECIAL as i32 {
                c = to_special(
                    i32::from(rem.get(1).copied().unwrap_or(0)),
                    i32::from(rem.get(2).copied().unwrap_or(0)),
                );
                rem = &rem[2.min(rem.len())..];
            }
            if is_special(c) || modifiers != 0 {
                if fd.puts(&get_special_key_name(c, modifiers)) < 0 {
                    return false;
                }
                if !consumed_unescaped && !rem.is_empty() {
                    rem = &rem[1..];
                }
                continue;
            }
        }

        if c == NL {
            if what == 2 {
                if fd.puts(c"\\\x16\n") < 0 {
                    return false;
                }
            } else if fd.puts(c"<NL>") < 0 {
                return false;
            }
            rem = &rem[1..];
            continue;
        }

        if what == 2 && (ascii_iswhite(c) || c == b'"' as i32 || c == b'\\' as i32) {
            if fd.putc(b'\\' as i32) < 0 {
                return false;
            }
        } else if (c < b' ' as i32
            || c > b'~' as i32
            || c == b'|' as i32
            || (what == 0 && c == b' ' as i32)
            || (what == 1 && is_start && c == b' ' as i32)
            || (what != 2 && c == b'<' as i32))
            && fd.putc(CTRL_V_CONST) < 0
        {
            return false;
        }
        if fd.putc(c) < 0 {
            return false;
        }
        if !rem.is_empty() {
            rem = &rem[1..];
        }
    }
    true
}

#[derive(Clone, Copy)]
pub struct MapLoc {
    pub local: bool,
    pub abbr: bool,
    pub bucket: usize,
    pub idx: usize,
}

/// Check the string "keys" against the lhs of all mappings.
pub fn check_map(
    keys: &CStr,
    mode: i32,
    exact: i32,
    ign_mod: i32,
    abbr: i32,
    mut mp_out: Option<&mut Option<MapLoc>>,
    local_ptr: &mut i32,
    rhs_lua: &mut i32,
) -> Option<CString> {
    let keys_bytes = keys.to_bytes();
    let len = keys_bytes.len() as i32;
    *rhs_lua = LUA_NOREF;

    for local in (0..=1).rev() {
        let b = if local != 0 {
            g_st().curbuf.opt()
        } else {
            None
        };
        let found = with_map_buckets(b, abbr != 0, |buckets| {
            for (b_idx, bucket) in buckets.iter().enumerate() {
                for (i_idx, mp) in bucket.iter().enumerate() {
                    if (mp.m_mode & mode) != 0 && (exact == 0 || mp.m_keylen == len) {
                        let mut s_bytes = mp.m_keys.as_bytes_with_nul();
                        let mut keylen = mp.m_keylen;
                        if ign_mod != 0
                            && keylen >= 3
                            && s_bytes.starts_with(&[K_SPECIAL, KS_MODIFIER])
                        {
                            s_bytes = &s_bytes[3..];
                            keylen -= 3;
                        }
                        let minlen = keylen.min(len).max(0) as usize;
                        if s_bytes[..minlen.min(s_bytes.len())] == keys_bytes[..minlen] {
                            let loc = MapLoc {
                                local: local != 0,
                                abbr: abbr != 0,
                                bucket: b_idx,
                                idx: i_idx,
                            };
                            let rhs = (mp.m_luaref == LUA_NOREF).then(|| mp.m_str.clone());
                            return Some((loc, mp.m_luaref, rhs));
                        }
                    }
                }
            }
            None
        });
        if let Some((loc, luaref, rhs)) = found {
            if let Some(ref mut out) = mp_out {
                **out = Some(loc);
            }
            *local_ptr = local;
            *rhs_lua = luaref;
            return rhs;
        }
    }

    None
}

/// "hasmapto()" function
pub fn f_hasmapto(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let name = tv_get_string(&argvars[0]);
    let mut abbr = false;
    let mut buf = [0u8; NUMBUFLEN];
    let mode = if argvars[1].v_type == VarType::VarUnknown {
        c"nvo"
    } else {
        let m = tv_get_string_buf(&argvars[1], &mut buf);
        if argvars[2].v_type != VarType::VarUnknown {
            abbr = tv_get_bool(&argvars[2]) != 0;
        }
        m
    };

    rettv.vval =
        crate::eval::typval::TypvalVval::Number(i64::from(map_to_exists(&name, mode, abbr)));
}

fn mapblock_fill_dict(
    mp: &Mapblock,
    lhsrawalt: Option<&CStr>,
    buffer_value: i32,
    abbr: bool,
    compatible: bool,
    arena: &mut Arena,
) -> Dict {
    let mut dict = arena_dict(arena, 20);
    let lhs = str2special_save(
        mp.m_keys.as_bytes(),
        compatible,
        if compatible { K_FALSE } else { K_NONE },
    );
    let mut mapmode = [0u8; 7];
    map_mode_to_chars(mp.m_mode, &mut mapmode);
    let noremap_value = if compatible {
        i64::from(mp.m_noremap != 0)
    } else if mp.m_noremap == REMAP_SCRIPT {
        2
    } else {
        i64::from(mp.m_noremap != 0)
    };

    if mp.m_luaref != LUA_NOREF {
        put_c(
            &mut dict,
            c"callback",
            luaref_obj(api_new_luaref(mp.m_luaref)),
        );
    } else if compatible {
        put_c(
            &mut dict,
            c"rhs",
            cstr_to_arena_obj(arena, Some(mp.m_orig_str.as_c_str())),
        );
    } else {
        let rhs_cstr = str2special_save(mp.m_str.as_bytes(), false, K_NONE);
        put_c(&mut dict, c"rhs", cstr_to_arena_obj(arena, Some(&rhs_cstr)));
    }
    if let Some(ref desc) = mp.m_desc {
        put_c(&mut dict, c"desc", cstr_to_arena_obj(arena, Some(desc)));
    }
    put_c(&mut dict, c"lhs", cstr_to_arena_obj(arena, Some(&lhs)));
    put_c(
        &mut dict,
        c"lhsraw",
        cstr_to_arena_obj(arena, Some(&*mp.m_keys)),
    );
    if let Some(lhsrawalt) = lhsrawalt {
        let alt_s = crate::api::private::helpers::arena_bytes(arena, lhsrawalt.to_bytes());
        put_c(&mut dict, c"lhsrawalt", string_obj(alt_s));
    }
    put_c(&mut dict, c"noremap", integer_obj(noremap_value));
    put_c(
        &mut dict,
        c"script",
        integer_obj(i64::from(mp.m_noremap == REMAP_SCRIPT)),
    );
    put_c(&mut dict, c"expr", integer_obj(i64::from(mp.m_expr != 0)));
    put_c(
        &mut dict,
        c"silent",
        integer_obj(i64::from(mp.m_silent != 0)),
    );
    put_c(
        &mut dict,
        c"sid",
        integer_obj(i64::from(mp.m_script_ctx.sc_sid)),
    );
    put_c(&mut dict, c"scriptversion", integer_obj(1));
    put_c(
        &mut dict,
        c"lnum",
        integer_obj(i64::from(mp.m_script_ctx.sc_lnum)),
    );
    put_c(&mut dict, c"buffer", integer_obj(i64::from(buffer_value)));
    if !compatible {
        put_c(&mut dict, c"buf", integer_obj(i64::from(buffer_value)));
    }
    put_c(
        &mut dict,
        c"nowait",
        integer_obj(i64::from(mp.m_nowait != 0)),
    );
    put_c(
        &mut dict,
        c"replace_keycodes",
        integer_obj(i64::from(mp.m_replace_keycodes)),
    );
    put_c(
        &mut dict,
        c"mode",
        string_obj(crate::api::private::helpers::arena_bytes(
            arena,
            crate::memory::bytes_until_nul(&mapmode),
        )),
    );
    put_c(&mut dict, c"abbr", integer_obj(i64::from(abbr)));
    put_c(&mut dict, c"mode_bits", integer_obj(i64::from(mp.m_mode)));

    dict
}

fn get_maparg(argvars: &mut [TypVal], rettv: &mut TypVal, exact: i32) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;

    let keys = tv_get_string(&argvars[0]);
    if keys.to_bytes().is_empty() {
        return;
    }

    let mut buf = [0u8; NUMBUFLEN];
    let mut abbr = false;
    let mut get_dict = false;

    let which_cstr = if argvars[1].v_type != VarType::VarUnknown {
        let Some(w) = tv_get_string_buf_chk(&argvars[1], &mut buf) else {
            return;
        };
        if argvars[2].v_type != VarType::VarUnknown {
            abbr = tv_get_bool(&argvars[2]) != 0;
            if argvars[3].v_type != VarType::VarUnknown {
                get_dict = tv_get_bool(&argvars[3]) != 0;
            }
        }
        w
    } else {
        c""
    };

    let mut did_simplify = false;
    let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
    let mut which_bytes = which_cstr.to_bytes();
    let mode = get_map_mode(&mut which_bytes, false);

    let keys_bytes = keys.to_bytes();
    let p_cpo = opt_vars().p_cpo.to_owned();
    let cpo_bytes = p_cpo.as_bytes();
    let keys_simplified = replace_termcodes(
        keys_bytes,
        false,
        0,
        flags,
        Some(&mut did_simplify),
        cpo_bytes,
    )
    .unwrap_or_default();
    let mut mp_opt: Option<MapLoc> = None;
    let mut buffer_local: i32 = 0;
    let mut rhs_lua: LuaRef = LUA_NOREF;
    let mut rhs = check_map(
        keys_simplified.as_c_str(),
        mode,
        exact,
        0,
        i32::from(abbr),
        Some(&mut mp_opt),
        &mut buffer_local,
        &mut rhs_lua,
    );
    let alt_keys;
    if did_simplify {
        alt_keys = replace_termcodes(
            keys_bytes,
            false,
            0,
            flags | REPTERM_NO_SIMPLIFY,
            None,
            cpo_bytes,
        )
        .unwrap_or_default();
        rhs = check_map(
            alt_keys.as_c_str(),
            mode,
            exact,
            0,
            i32::from(abbr),
            Some(&mut mp_opt),
            &mut buffer_local,
            &mut rhs_lua,
        );
    }

    if !get_dict {
        if let Some(rhs) = rhs {
            if rhs.is_empty() {
                crate::eval::typval::tv_set_string(&mut *rettv, Some(c"<Nop>"));
            } else {
                let special = str2special_save(rhs.to_bytes(), false, K_FALSE);
                crate::eval::typval::tv_set_string(&mut *rettv, Some(special.as_c_str()));
            }
        } else if rhs_lua != LUA_NOREF && mp_opt.is_some() {
            let s = nlua_funcref_str(rhs_lua, true);
            crate::eval::typval::tv_set_string_len(&mut *rettv, Some(s.as_bytes()));
        }
    } else if let Some(loc) = mp_opt
        && (rhs.is_some() || rhs_lua != LUA_NOREF)
    {
        let mut arena = Arena::new();
        let b = if loc.local { g_st().curbuf.opt() } else { None };
        let dict = with_map_buckets(b, loc.abbr, |buckets| {
            let mp = &buckets[loc.bucket][loc.idx];
            mapblock_fill_dict(
                mp,
                did_simplify.then_some(keys_simplified.as_c_str()),
                buffer_local,
                abbr,
                true,
                &mut arena,
            )
        });
        let mut dobj = dict_obj(dict);
        object_to_vim_take_luaref(
            &mut dobj,
            &mut *rettv,
            true,
            &mut crate::api::private::validate::Error::new(),
        );
        arena_mem_free(arena_finish(&mut arena));
    } else {
        tv_dict_alloc_ret(rettv);
    }
}

fn get_map_mode_string(mode_string: &[u8], abbr: bool) -> i32 {
    const MASK_V: i32 = MODE_VISUAL | MODE_SELECT;
    const MASK_MAP: i32 = MODE_VISUAL | MODE_SELECT | MODE_NORMAL | MODE_OP_PENDING;
    const MASK_BANG: i32 = MODE_INSERT | MODE_CMDLINE;

    let bytes = if mode_string.is_empty() {
        b" "
    } else {
        mode_string
    };
    let mut mode: i32 = 0;
    for &modec in bytes {
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

/// "mapset()" function
pub fn f_mapset(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    if check_secure() {
        return;
    }

    let mut buf = [0u8; NUMBUFLEN];
    let dict_only = argvars[0].v_type == VarType::VarDict;

    let (which, is_abbr, d) = if dict_only {
        let d = argvars[0].vval.v_dict();
        let d_opt = d.opt();
        let which = tv_dict_get_string(d_opt, c"mode");
        let is_abbr = tv_dict_get_bool(d_opt, c"abbr", -1) as i32;
        let Some(which) = which else {
            emsg(gettext(E_ENTRIES_MISSING_IN_MAPSET_DICT_ARGUMENT));
            return;
        };
        if is_abbr < 0 {
            emsg(gettext(E_ENTRIES_MISSING_IN_MAPSET_DICT_ARGUMENT));
            return;
        }
        (which, is_abbr, d)
    } else {
        let Some(which) = tv_get_string_buf_chk(&argvars[0], &mut buf) else {
            return;
        };
        let is_abbr = tv_get_bool(&argvars[1]) as i32;
        if !tv_check_for_dict_arg(argvars, 2) {
            return;
        }
        let d = argvars[2].vval.v_dict();
        (std::borrow::Cow::Borrowed(which), is_abbr, d)
    };
    let mode = get_map_mode_string(which.to_bytes(), is_abbr != 0);
    if mode == 0 {
        semsg!(gettext(E_ILLEGAL_MAP_MODE_STRING_STR), which);
        return;
    }

    let d_opt = d.opt();
    let lhs = crate::eval::typval::tv_dict_get_cstring(d_opt, c"lhs");
    let lhsraw = crate::eval::typval::tv_dict_get_cstring(d_opt, c"lhsraw");
    let lhsrawalt = crate::eval::typval::tv_dict_get_cstring(d_opt, c"lhsrawalt");
    let mut orig_rhs = crate::eval::typval::tv_dict_get_cstring(d_opt, c"rhs");
    let mut rhs_lua: LuaRef = LUA_NOREF;
    if let Some(callback_di) = tv_dict_find(d_opt, c"callback", 8)
        && callback_di.tv().v_type == VarType::VarFunc
        && let Some(fp) = callback_di
            .tv()
            .vval
            .v_string_cstr()
            .and_then(|s| find_func(Some(s)))
        && (fp.uf_flags.get() & FC_LUAREF) != 0
    {
        rhs_lua = api_new_luaref(fp.uf_luaref.get());

        orig_rhs = Some(std::ffi::CString::default());
    }
    let (Some(lhs), Some(lhsraw), Some(orig_rhs)) = (lhs, lhsraw, orig_rhs) else {
        emsg(gettext(E_ENTRIES_MISSING_IN_MAPSET_DICT_ARGUMENT));
        api_free_luaref(rhs_lua);
        return;
    };

    let mut noremap = if tv_dict_get_number(d_opt, c"noremap") != 0 {
        REMAP_NONE
    } else {
        0
    };
    if tv_dict_get_number(d_opt, c"script") != 0 {
        noremap = REMAP_SCRIPT;
    }
    let mut args = MapArguments::new();

    args.expr = tv_dict_get_number(d_opt, c"expr") != 0;

    args.silent = tv_dict_get_number(d_opt, c"silent") != 0;

    args.nowait = tv_dict_get_number(d_opt, c"nowait") != 0;

    args.replace_keycodes = tv_dict_get_number(d_opt, c"replace_keycodes") != 0;

    args.desc = crate::eval::typval::tv_dict_get_cstring(d_opt, c"desc");

    let sid = tv_dict_get_number(d_opt, c"sid") as Scid;
    let lnum = tv_dict_get_number(d_opt, c"lnum") as Linenr;
    let buffer = tv_dict_get_number(d_opt, c"buffer") != 0;

    let p_cpo = opt_vars().p_cpo.to_owned();
    let cpo_bytes = p_cpo.as_bytes();
    set_maparg_rhs(orig_rhs.as_bytes(), rhs_lua, sid, cpo_bytes, &mut args);

    let mut unmap_args = MapArguments::new();
    set_maparg_lhs_rhs(lhs.as_bytes(), b"", LUA_NOREF, cpo_bytes, &mut unmap_args);
    unmap_args.buffer = buffer;
    buf_do_map(
        MAPTYPE_UNMAP_LHS,
        &mut unmap_args,
        mode,
        is_abbr != 0,
        curbuf_mut(),
    );

    let alt = lhsrawalt.is_some().then(|| {
        std::rc::Rc::new(MapAltLink {
            keys: [std::cell::Cell::new(None), std::cell::Cell::new(None)],
        })
    });
    map_add(
        curbuf_mut(),
        buffer,
        lhsraw.as_c_str(),
        &args,
        noremap,
        mode,
        is_abbr != 0,
        sid,
        lnum,
        false,
        alt.clone(),
    );
    if let Some(ref lhsrawalt) = lhsrawalt {
        map_add(
            curbuf_mut(),
            buffer,
            lhsrawalt.as_c_str(),
            &args,
            noremap,
            mode,
            is_abbr != 0,
            sid,
            lnum,
            true,
            alt,
        );
    }
}

/// "maplist()" function
pub fn f_maplist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
    let abbr = argvars[0].v_type != VarType::VarUnknown && tv_get_bool(&argvars[0]) != 0;

    tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN_CONST);

    for buffer_local in 0..=1 {
        let b = if buffer_local != 0 {
            g_st().curbuf.opt()
        } else {
            None
        };
        with_map_buckets(b, abbr, |buckets| {
            for bucket in buckets {
                for mp in bucket {
                    if mp.m_simplified != 0 {
                        continue;
                    }

                    let mut did_simplify = false;

                    let mut arena = Arena::new();
                    let lhs = str2special_save(mp.m_keys.as_bytes(), true, K_FALSE);
                    let keys_buf = replace_termcodes(
                        lhs.to_bytes(),
                        false,
                        0,
                        flags,
                        Some(&mut did_simplify),
                        opt_vars().p_cpo.as_bytes(),
                    )
                    .unwrap_or_default();

                    let dict = mapblock_fill_dict(
                        mp,
                        did_simplify.then_some(keys_buf.as_c_str()),
                        buffer_local,
                        abbr,
                        true,
                        &mut arena,
                    );
                    let mut d = TypVal {
                        v_type: VarType::VarUnknown,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::Number(0),
                    };
                    let mut dobj = dict_obj(dict);
                    object_to_vim_take_luaref(
                        &mut dobj,
                        &mut d,
                        true,
                        &mut crate::api::private::validate::Error::new(),
                    );
                    debug_assert!(d.v_type == VarType::VarDict);
                    tv_list_append_dict(rettv.vval.v_list_ref(), d.vval.v_dict_opt());
                    arena_mem_free(arena_finish(&mut arena));
                }
            }
        });
    }
}

/// "maparg()" function
pub fn f_maparg(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    get_maparg(argvars, rettv, 1);
}

/// "mapcheck()" function
pub fn f_mapcheck(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    get_maparg(argvars, rettv, 0);
}

/// Add a mapping.
pub fn add_map(lhs: &std::ffi::CStr, rhs: &std::ffi::CStr, mode: i32, buffer: bool) {
    let mut args = MapArguments::new();
    set_maparg_lhs_rhs(
        lhs.to_bytes(),
        rhs.to_bytes(),
        LUA_NOREF,
        opt_vars().p_cpo.as_bytes(),
        &mut args,
    );
    args.buffer = buffer;

    buf_do_map(MAPTYPE_NOREMAP, &mut args, mode, false, curbuf_mut());
}

fn langmap_set_entry(from: i32, to: i32) {
    LANGMAP_MAPGA.with(|cell| {
        let mut entries = cell.borrow_mut();
        match entries.binary_search_by_key(&from, |e| e.from) {
            Ok(idx) => entries[idx].to = to,
            Err(idx) => entries.insert(idx, LangmapEntry { from, to }),
        }
    });
}

/// Apply 'langmap' to multi-byte character "c" and return the result.
pub fn langmap_adjust_mb(c: i32) -> i32 {
    LANGMAP_MAPGA.with(|cell| {
        let entries = cell.borrow();
        entries
            .binary_search_by_key(&c, |e| e.from)
            .map_or(c, |idx| entries[idx].to)
    })
}

pub fn langmap_init() {
    for (i, slot) in g_st().langmap_mapchar.iter_mut().enumerate() {
        *slot = i as u8;
    }
    LANGMAP_MAPGA.with(|cell| cell.borrow_mut().clear());
}

/// Called when langmap option is set; the language map can be
/// changed at any time!
pub fn did_set_langmap<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    langmap_init();

    let p_langmap = opt_vars().p_langmap.to_owned();
    let base_cstr = p_langmap.as_c_str();
    let bytes = base_cstr.to_bytes();
    let mut p_idx = 0usize;
    while p_idx < bytes.len() {
        let mut p2_idx = p_idx;
        while p2_idx < bytes.len() && !matches!(bytes[p2_idx], b',' | b';') {
            if bytes[p2_idx] == b'\\' && p2_idx + 1 < bytes.len() {
                p2_idx += 1;
            }
            p2_idx += utfc_ptr2len(&bytes[p2_idx..]) as usize;
        }
        let mut p2_opt = if bytes.get(p2_idx) == Some(&b';') {
            Some(p2_idx + 1)
        } else {
            None
        };
        while p_idx < bytes.len() {
            if bytes[p_idx] == b',' {
                p_idx += 1;
                break;
            }
            if bytes[p_idx] == b'\\' && p_idx + 1 < bytes.len() {
                p_idx += 1;
            }
            let from_slice = &bytes[p_idx..];
            let from = utf_ptr2char(from_slice);
            let mut to: i32 = NUL as i32;
            let mut to_cstr = c"";
            if let Some(ref mut p2) = p2_opt {
                if bytes.get(*p2) != Some(&b',') && *p2 < bytes.len() {
                    if bytes[*p2] == b'\\' && *p2 + 1 < bytes.len() {
                        *p2 += 1;
                    }
                    to_cstr = &base_cstr[*p2..];
                    to = utf_ptr2char(to_cstr.to_bytes());
                }
            } else {
                p_idx += utfc_ptr2len(from_slice) as usize;
                if p_idx < bytes.len() && bytes[p_idx] != b',' {
                    if bytes[p_idx] == b'\\' && p_idx + 1 < bytes.len() {
                        p_idx += 1;
                    }
                    to_cstr = &base_cstr[p_idx..];
                    to = utf_ptr2char(to_cstr.to_bytes());
                }
            }
            if to == NUL as i32 {
                return opt_error(
                    args.os_errbuf.take(),
                    c"E357: 'langmap': Matching character missing for %s",
                    &transchar(from),
                );
            }

            if from >= 256 {
                langmap_set_entry(from, to);
            } else {
                if to > UCHAR_MAX {
                    let from_cstr = &base_cstr[bytes.len() - from_slice.len()..];
                    swmsg!(
                        true,
                        c"'langmap': Mapping from %.*s to %.*s will not work properly",
                        utf_ptr2len(from_slice),
                        from_cstr,
                        utf_ptr2len(to_cstr.to_bytes()),
                        to_cstr,
                    );
                }
                g_st().langmap_mapchar[(from & 255) as usize] = to as u8;
            }

            p_idx += utfc_ptr2len(&bytes[p_idx..]) as usize;
            if let Some(ref mut p2) = p2_opt {
                *p2 += utfc_ptr2len(&bytes[*p2..]) as usize;
                if bytes.get(p_idx) == Some(&b';') {
                    p_idx = *p2;
                    if p_idx < bytes.len() {
                        if bytes[p_idx] != b',' {
                            return opt_error(
                                args.os_errbuf.take(),
                                c"E358: 'langmap': Extra characters after semicolon: %s",
                                &base_cstr[p_idx..],
                            );
                        }
                        p_idx += 1;
                    }
                    break;
                }
            }
        }
    }

    None
}

fn do_exmap(eap: &mut Exarg, isabbrev: bool) {
    let mut cmd_bytes = eap.cmd.as_bytes();
    let mode = get_map_mode(&mut cmd_bytes, eap.forceit || isabbrev);

    let maptype = if cmd_bytes.first() == Some(&b'n') {
        MAPTYPE_NOREMAP
    } else if cmd_bytes.first() == Some(&b'u') {
        MAPTYPE_UNMAP
    } else {
        MAPTYPE_MAP
    };
    let mut parsed_args = MapArguments::new();
    let result = str_to_mapargs(
        eap.arg.as_cstr(),
        maptype == MAPTYPE_UNMAP,
        &mut parsed_args,
    );
    match result {
        0 => match buf_do_map(maptype, &mut parsed_args, mode, isabbrev, curbuf_mut()) {
            1 => {
                emsg(gettext(E_INVARG));
            }
            2 => {
                emsg(if isabbrev {
                    gettext(E_NOABBR)
                } else {
                    gettext(E_NOMAP)
                });
            }
            5 => {
                semsg!(
                    if isabbrev {
                        gettext(E_ABBREVIATION_ALREADY_EXISTS_FOR_STR)
                    } else {
                        gettext(E_MAPPING_ALREADY_EXISTS_FOR_STR)
                    },
                    &parsed_args.lhs,
                );
            }
            6 => {
                semsg!(
                    if isabbrev {
                        gettext(E_GLOBAL_ABBREVIATION_ALREADY_EXISTS_FOR_STR)
                    } else {
                        gettext(E_GLOBAL_MAPPING_ALREADY_EXISTS_FOR_STR)
                    },
                    &parsed_args.lhs,
                );
            }
            _ => {}
        },
        1 => {
            emsg(gettext(E_INVARG));
        }
        _ => {
            debug_assert!(false, "Unknown return code from str_to_mapargs!");
        }
    }
}

/// ":abbreviate" and friends.
pub fn ex_abbreviate(eap: &mut Exarg) {
    do_exmap(eap, true);
}

/// ":map" and friends.
pub fn ex_map(eap: &mut Exarg) {
    if g_st().secure != 0 {
        g_st().secure = 2;
        msg_outtrans(eap.cmd.as_cstr(), 0, false);
        msg_putchar(b'\n' as i32);
    }
    do_exmap(eap, false);
}

/// ":unmap" and friends.
pub fn ex_unmap(eap: &mut Exarg) {
    do_exmap(eap, false);
}

/// ":mapclear" and friends.
pub fn ex_mapclear(eap: &mut Exarg) {
    do_mapclear(eap.cmd.as_cstr(), eap.arg.as_cstr(), eap.forceit, false);
}

/// ":abclear" and friends.
pub fn ex_abclear(eap: &mut Exarg) {
    do_mapclear(eap.cmd.as_cstr(), eap.arg.as_cstr(), true, true);
}

/// Set, tweak, or remove a mapping in a mode.
pub fn modify_keymap(
    channel_id: u64,
    mut buffer: Buffer,
    mut maptype: i32,
    mode: NvimString,
    lhs: NvimString,
    rhs: NvimString,
    opts: Option<&mut KeyDict_keymap>,
    err: &mut Error,
) {
    let mut lua_funcref: LuaRef = LUA_NOREF;
    let is_unmap = maptype == MAPTYPE_UNMAP || maptype == MAPTYPE_UNMAP_LHS;
    let global = buffer == -1;
    if global {
        buffer = 0;
    }
    let Some(target_buf) = crate::api::private::helpers::find_buffer_by_handle_mut(buffer, err)
    else {
        return;
    };

    let save_current_sctx = api_set_sctx(channel_id);

    let mut parsed_args = MapArguments::new();
    if let Some(opts) = opts {
        parsed_args.nowait = opts.nowait;
        parsed_args.noremap = opts.noremap;
        parsed_args.silent = opts.silent;
        parsed_args.script = opts.script;
        parsed_args.expr = opts.expr;
        parsed_args.unique = opts.unique;
        parsed_args.replace_keycodes = opts.replace_keycodes;
        if has_key(opts.is_set_keymap, KEYSET_OPTIDX_KEYMAP_CALLBACK) {
            lua_funcref = opts.callback;
            opts.callback = LUA_NOREF;
        }
        if has_key(opts.is_set_keymap, KEYSET_OPTIDX_KEYMAP_DESC) {
            parsed_args.desc = Some(string_to_cstr(opts.desc.clone()));
        }
    }
    parsed_args.buffer = !global;

    'block: {
        if parsed_args.replace_keycodes && !parsed_args.expr {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"\"replace_keycodes\" requires \"expr\"",
            );
            break 'block;
        }

        if !set_maparg_lhs_rhs(
            lhs.as_bytes(),
            rhs.as_bytes(),
            lua_funcref,
            opt_vars().p_cpo.as_bytes(),
            &mut parsed_args,
        ) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"LHS exceeds maximum map length: %s",
                lhs,
            );
            break 'block;
        }

        if parsed_args.lhs_len > MAXMAPLEN || parsed_args.alt_lhs_len > MAXMAPLEN {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"LHS exceeds maximum map length: %s",
                lhs,
            );
            break 'block;
        }

        let mut p_slice: &[u8] = if mode.size > 0 { mode.as_bytes() } else { b"m" };
        let forceit = p_slice.first() == Some(&b'!');
        let mode_val = get_map_mode(&mut p_slice, forceit);
        if forceit {
            p_slice = &p_slice[1..];
        }
        let is_abbrev =
            (mode_val & (MODE_INSERT | MODE_CMDLINE)) != 0 && p_slice.first() == Some(&b'a');
        if is_abbrev {
            p_slice = &p_slice[1..];
        }
        if mode.size > 0 && !p_slice.is_empty() {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Invalid mode shortname: \"%s\"",
                mode,
            );
            break 'block;
        }

        if parsed_args.lhs_len == 0 {
            api_set_error!(err, ErrorType::ErrorTypeValidation, c"Invalid (empty) LHS",);
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
                    ErrorType::ErrorTypeValidation,
                    c"Gave nonempty RHS in unmap command: %s",
                    parsed_args.rhs.as_deref().unwrap_or(c""),
                );
            } else {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"Gave nonempty RHS for unmap",
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
                api_set_error!(err, ErrorType::ErrorTypeException, E_INVARG, 0);
            }
            2 => {
                api_set_error!(err, ErrorType::ErrorTypeException, E_NOMAP, 0);
            }
            5 => {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    if is_abbrev {
                        E_ABBREVIATION_ALREADY_EXISTS_FOR_STR
                    } else {
                        E_MAPPING_ALREADY_EXISTS_FOR_STR
                    },
                    lhs,
                );
            }
            6 => {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    if is_abbrev {
                        E_GLOBAL_ABBREVIATION_ALREADY_EXISTS_FOR_STR
                    } else {
                        E_GLOBAL_MAPPING_ALREADY_EXISTS_FOR_STR
                    },
                    lhs,
                );
            }
            _ => {
                debug_assert!(false, "Unrecognized return code!");
            }
        }
    }

    g_st().current_sctx = save_current_sctx;
    nlua_clear_ref(&mut parsed_args.rhs_lua);
}

/// Get an array containing dictionaries describing mappings
/// based on mode and buffer id.
pub fn keymap_array(mode: NvimString, buf: Option<&FileBuf>, arena: &mut Arena) -> Array {
    let mut mappings = ArrayBuilder::new();

    let mut p_slice: &[u8] = if mode.size > 0 { mode.as_bytes() } else { b"m" };
    let forceit = p_slice.first() == Some(&b'!');
    let int_mode = get_map_mode(&mut p_slice, forceit);
    if forceit {
        p_slice = &p_slice[1..];
    }
    let is_abbrev =
        (int_mode & (MODE_INSERT | MODE_CMDLINE)) != 0 && p_slice.first() == Some(&b'a');

    let buffer_value = buf.map_or(0, |b| b.b_fnum);

    with_map_buckets(buf, is_abbrev, |buckets| {
        for bucket in buckets {
            for current_maphash in bucket {
                if current_maphash.m_simplified == 0 && (int_mode & current_maphash.m_mode) != 0 {
                    let alt_cstr = current_maphash.alt_keys();
                    mappings.push(dict_obj(mapblock_fill_dict(
                        current_maphash,
                        alt_cstr.as_deref(),
                        buffer_value,
                        is_abbrev,
                        false,
                        arena,
                    )));
                }
            }
        }
    });

    arena_take_arraybuilder(arena, &mut mappings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_mode_to_chars_and_langmap_init_work() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            let mut buf = [0u8; 16];
            map_mode_to_chars(MODE_NORMAL | MODE_VISUAL, &mut buf);
            assert_eq!(crate::memory::bytes_until_nul(&buf), b"nx");

            langmap_init();
            assert_eq!(langmap_adjust_mb(b'a' as i32), b'a' as i32);
        }
    }
}
