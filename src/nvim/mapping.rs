//! Code for mappings and abbreviations (`src/nvim/mapping.c`).

use crate::api::buffer::KeyDict_keymap;
use crate::api::private::converter::object_to_vim_take_luaref;
use crate::api::private::helpers::{
    ArrayBuilder, api_clear_error, api_free_object, api_set_error, api_set_sctx, arena_dict,
    arena_take_arraybuilder, cstr_as_obj, cstr_as_string, dict_obj, has_key, integer_obj,
    luaref_obj, put_c, string_obj, string_to_cstr,
};
use crate::api::private::validate::{Array, Dict, Error, ErrorType, NvimString, ObjectType};
use crate::api::tabpage::Buffer;
use crate::buffer::FileBuf;
use crate::charset::{
    NUL, ascii_isspace, ascii_iswhite, is_special, skipwhite, to_special, transchar, vim_iswordp,
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
use crate::fileio::{FILE, fputc, fputs};
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
    mb_prevptr, mb_unescape, utf_char2bytes, utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::memory::{Arena, arena_alloc, arena_finish, arena_mem_free, xstrlcpy};
use crate::message::{
    emsg, iemsg, message_filtered, msg, msg_clr_eos, msg_ext_set_kind, msg_outtrans,
    msg_outtrans_special, msg_putchar, msg_puts, msg_puts_hl, msg_start, semsg, semsg_multiline,
    str2special_arena, str2special_save, swmsg,
};
use crate::option::opt_error;
use crate::option_vars::*;
use crate::optionstr::Optset;
use crate::os::env::ExpandCtx;
use crate::regexp::{CTRL_V_CONST, NUMBUFLEN, Regmatch, vim_regexec};
use crate::runtime::{Sctx, sourcing_lnum};
use crate::strings::{gettext, sort_strings, vim_snprintf};
use crate::viml::parser::expressions::CTRL_H_CONST;
use std::ffi::{CStr, CString};
use std::ptr;

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

struct MappingModuleState {
    first_abbr: Vec<Mapblock>,
    maphash: [Vec<Mapblock>; MAX_MAPHASH],
    map_luaid: i32,
    expand_mapmodes: i32,
    expand_isabbrev: bool,
    expand_buffer: bool,
    langmap_mapga: Vec<LangmapEntry>,
}

impl MappingModuleState {
    const fn new() -> Self {
        Self {
            first_abbr: Vec::new(),
            maphash: [const { Vec::new() }; MAX_MAPHASH],
            map_luaid: 0,
            expand_mapmodes: 0,
            expand_isabbrev: false,
            expand_buffer: false,
            langmap_mapga: Vec::new(),
        }
    }
}

static MAP_STATE: GlobalState<MappingModuleState> = GlobalState::new(MappingModuleState::new());

#[inline]
fn map_st() -> &'static mut MappingModuleState {
    unsafe { &mut *MAP_STATE.as_ptr() }
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

/// Get the start of the hashed map list for "state" and first character "c".
pub fn get_maphash_list(state: i32, c: i32) -> &'static [Mapblock] {
    unsafe { (&raw const map_st().maphash[map_hash(state, c as u8)]).as_ref() }
        .unwrap()
        .as_slice()
}

/// Get the buffer-local hashed map list for "state" and first character "c".
pub fn get_buf_maphash_list(state: i32, c: i32) -> &'static [Mapblock] {
    curbuf_mut().b_maphash[map_hash(state, c as u8)].as_slice()
}

#[inline]
fn get_map_buckets(buf: *const FileBuf, is_abbrev: bool) -> &'static [Vec<Mapblock>] {
    if is_abbrev {
        if !buf.is_null() {
            std::slice::from_ref(unsafe { &(*buf).b_first_abbr })
        } else {
            std::slice::from_ref(&map_st().first_abbr)
        }
    } else if !buf.is_null() {
        unsafe { &(*buf).b_maphash }
    } else {
        &map_st().maphash
    }
}

/// Finds the Lua mapping whose `m_str` embeds `id`.
fn map_luaid_scan(mappings: &[Vec<Mapblock>], id: i32) -> Option<&Mapblock> {
    for bucket in mappings {
        for mp in bucket {
            if mp.m_luaref != LUA_NOREF
                && crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(
                    mp.m_str.as_ptr().wrapping_add(3),
                )) as i32
                    == id
            {
                return Some(mp);
            }
        }
    }
    None
}

/// Resolves a Lua-mapping id to its callback.
pub fn map_luaid_get(n: i32) -> LuaRef {
    let maphash_ref = &map_st().maphash;
    let first_abbr_ref = &map_st().first_abbr;
    let mut mp = map_luaid_scan(maphash_ref, n);
    if mp.is_none() {
        mp = map_luaid_scan(std::slice::from_ref(first_abbr_ref), n);
    }
    for fb in for_all_buffers_mut() {
        if mp.is_none() {
            mp = map_luaid_scan(&fb.b_maphash, n);
        }
        if mp.is_none() {
            mp = map_luaid_scan(std::slice::from_ref(&fb.b_first_abbr), n);
        }
    }
    mp.map_or(LUA_NOREF, |m| m.m_luaref)
}

/// Put characters to represent the map mode in a string buffer.
pub fn map_mode_to_chars<T>(mode: i32, buf: *mut T) {
    unsafe {
        let mut p = buf.cast::<u8>();
        if (mode & (MODE_INSERT | MODE_CMDLINE)) == (MODE_INSERT | MODE_CMDLINE) {
            *p = b'!';
            p = p.wrapping_add(1);
        } else if (mode & MODE_INSERT) != 0 {
            *p = b'i';
            p = p.wrapping_add(1);
        } else if (mode & MODE_LANGMAP) != 0 {
            *p = b'l';
            p = p.wrapping_add(1);
        } else if (mode & MODE_CMDLINE) != 0 {
            *p = b'c';
            p = p.wrapping_add(1);
        } else if (mode & (MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING))
            == (MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING)
        {
            *p = b' ';
            p = p.wrapping_add(1);
        } else {
            if (mode & MODE_NORMAL) != 0 {
                *p = b'n';
                p = p.wrapping_add(1);
            }
            if (mode & MODE_OP_PENDING) != 0 {
                *p = b'o';
                p = p.wrapping_add(1);
            }
            if (mode & MODE_TERMINAL) != 0 {
                *p = b't';
                p = p.wrapping_add(1);
            }
            if (mode & (MODE_VISUAL | MODE_SELECT)) == (MODE_VISUAL | MODE_SELECT) {
                *p = b'v';
                p = p.wrapping_add(1);
            } else {
                if (mode & MODE_VISUAL) != 0 {
                    *p = b'x';
                    p = p.wrapping_add(1);
                }
                if (mode & MODE_SELECT) != 0 {
                    *p = b's';
                    p = p.wrapping_add(1);
                }
            }
        }
        *p = 0;
    }
}

fn showmap(mp: &Mapblock, local: bool) {
    if message_filtered(mp.m_keys.as_ptr())
        && message_filtered(mp.m_str.as_ptr())
        && mp
            .m_desc
            .as_ref()
            .is_none_or(|d| message_filtered(d.as_ptr()))
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
    map_mode_to_chars(mp.m_mode, mapchars.as_mut_ptr());
    msg_puts(mapchars.as_ptr().cast());
    let mut len = crate::memory::cstr_len(mapchars.as_ptr().cast());

    loop {
        len += 1;
        if len > 3 {
            break;
        }
        msg_putchar(b' ' as i32);
    }

    len = msg_outtrans_special(mp.m_keys.as_ptr(), true, 0) as usize;
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
        msg_puts_hl(str_cstr.as_ptr(), HLF_8, false);
    } else if mp.m_str.as_bytes().is_empty() {
        msg_puts_hl(c"<Nop>", HLF_8, false);
    } else {
        msg_outtrans_special(mp.m_str.as_ptr(), false, 0);
    }

    if let Some(ref desc) = mp.m_desc {
        msg_puts(c"\n                 ");
        msg_puts(desc.as_ptr());
    }
    if opt_vars().p_verbose > 0 {
        last_set_msg(mp.m_script_ctx);
    }
    msg_clr_eos();
}

fn set_maparg_lhs_rhs(
    orig_lhs: impl crate::strings::IntoCCharPtr,
    orig_lhs_len: usize,
    orig_rhs: impl crate::strings::IntoCCharPtr,
    orig_rhs_len: usize,
    rhs_lua: LuaRef,
    cpo_val: impl crate::strings::IntoCCharPtr,
    mapargs: &mut MapArguments,
) -> bool {
    let orig_lhs = orig_lhs.into_c_char_ptr();
    let cpo_val = cpo_val.into_c_char_ptr();
    let orig_rhs = orig_rhs.into_c_char_ptr();
    mapargs.rhs_lua = rhs_lua;

    let mut did_simplify = false;
    let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
    let Some(replaced) = replace_termcodes(
        orig_lhs,
        orig_lhs_len,
        true,
        0,
        flags,
        Some(&mut did_simplify),
        cpo_val,
    ) else {
        return false;
    };
    mapargs.lhs_len = replaced.as_bytes().len();
    xstrlcpy(
        mapargs.lhs.as_mut_ptr().cast(),
        replaced.as_ptr(),
        mapargs.lhs.len(),
    );
    if did_simplify {
        let Some(alt_replaced) = replace_termcodes(
            orig_lhs,
            orig_lhs_len,
            true,
            0,
            flags | REPTERM_NO_SIMPLIFY,
            None,
            cpo_val,
        ) else {
            return false;
        };
        mapargs.alt_lhs_len = alt_replaced.as_bytes().len();
        xstrlcpy(
            mapargs.alt_lhs.as_mut_ptr().cast(),
            alt_replaced.as_ptr(),
            mapargs.alt_lhs.len(),
        );
    } else {
        mapargs.alt_lhs_len = 0;
    }

    set_maparg_rhs(orig_rhs, orig_rhs_len, rhs_lua, 0, cpo_val, mapargs);
    true
}

fn set_maparg_rhs(
    orig_rhs: impl crate::strings::IntoCCharPtr,
    orig_rhs_len: usize,
    rhs_lua: LuaRef,
    sid: Scid,
    cpo_val: impl crate::strings::IntoCCharPtr,
    mapargs: &mut MapArguments,
) {
    let orig_rhs = orig_rhs.into_c_char_ptr();
    let cpo_val = cpo_val.into_c_char_ptr();
    {
        mapargs.rhs_lua = rhs_lua;

        if rhs_lua == LUA_NOREF {
            mapargs.orig_rhs_len = orig_rhs_len;
            let orig_slice = crate::memory::cstr_slice(orig_rhs, orig_rhs_len);
            mapargs.orig_rhs = Some(CString::new(orig_slice).unwrap());
            if crate::memory::cstr_as_bytes(orig_rhs).eq_ignore_ascii_case(b"<nop>") {
                mapargs.rhs = Some(CString::default());
                mapargs.rhs_len = 0;
                mapargs.rhs_is_noop = true;
            } else {
                let replaced_cstr = replace_termcodes(
                    orig_rhs,
                    orig_rhs_len,
                    false,
                    sid,
                    REPTERM_DO_LT,
                    None,
                    cpo_val,
                )
                .unwrap_or_default();
                mapargs.rhs_len = replaced_cstr.as_bytes().len();
                mapargs.rhs_is_noop = orig_rhs_len != 0 && mapargs.rhs_len == 0;
                mapargs.rhs = Some(replaced_cstr);
            }
        } else {
            let mut tmp_buf = [0u8; 64];
            mapargs.orig_rhs = Some(CString::default());
            mapargs.orig_rhs_len = 0;
            map_st().map_luaid += 1;
            mapargs.rhs_len = vim_snprintf!(
                tmp_buf.as_mut_ptr().cast(),
                tmp_buf.len(),
                c"%c%c%c%d\r",
                K_SPECIAL as i32,
                KS_EXTRA as i32,
                KE_LUA as i32,
                map_st().map_luaid,
            ) as usize;
            mapargs.rhs = Some(crate::memory::cstr_to_cstring(tmp_buf.as_ptr().cast()));
        }
    }
}

fn str_to_mapargs(
    strargs: impl crate::strings::IntoCCharPtr,
    is_unmap: bool,
    mapargs: &mut MapArguments,
) -> i32 {
    let strargs = strargs.into_c_char_ptr();
    {
        let mut to_parse = skipwhite(strargs);
        *mapargs = MapArguments::new();
        mapargs.rhs_lua = 0;

        loop {
            let bytes = crate::memory::cstr_as_bytes(to_parse);
            if bytes.starts_with(b"<buffer>") {
                to_parse = skipwhite(to_parse.wrapping_add(8));
                mapargs.buffer = true;
                continue;
            }
            if bytes.starts_with(b"<nowait>") {
                to_parse = skipwhite(to_parse.wrapping_add(8));
                mapargs.nowait = true;
                continue;
            }
            if bytes.starts_with(b"<silent>") {
                to_parse = skipwhite(to_parse.wrapping_add(8));
                mapargs.silent = true;
                continue;
            }
            if bytes.starts_with(b"<special>") {
                to_parse = skipwhite(to_parse.wrapping_add(9));
                continue;
            }
            if bytes.starts_with(b"<script>") {
                to_parse = skipwhite(to_parse.wrapping_add(8));
                mapargs.script = true;
                continue;
            }
            if bytes.starts_with(b"<expr>") {
                to_parse = skipwhite(to_parse.wrapping_add(6));
                mapargs.expr = true;
                continue;
            }
            if bytes.starts_with(b"<unique>") {
                to_parse = skipwhite(to_parse.wrapping_add(8));
                mapargs.unique = true;
                continue;
            }
            break;
        }

        let mut lhs_end = to_parse;
        let do_backslash = !crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_BSLASH_CONST);
        while !crate::memory::cstr_is_empty(lhs_end)
            && (is_unmap || !ascii_iswhite(crate::memory::cstr_first_byte(lhs_end) as i32))
        {
            if (crate::memory::cstr_first_byte(lhs_end) == CTRL_V_CONST as u8
                || (do_backslash && crate::memory::cstr_first_byte(lhs_end) == b'\\'))
                && crate::memory::cstr_byte_at(lhs_end, 1) != 0
            {
                lhs_end = lhs_end.wrapping_add(1);
            }
            lhs_end = lhs_end.wrapping_add(1);
        }

        let rhs_start = skipwhite(lhs_end);

        let orig_lhs_len = (lhs_end as usize) - (to_parse as usize);
        if orig_lhs_len >= 256 {
            return 1;
        }
        let mut lhs_to_replace = [0u8; 256];
        crate::memory::cstr_ncpy(lhs_to_replace.as_mut_ptr().cast(), to_parse, orig_lhs_len);

        let orig_rhs_len = crate::memory::cstr_len(rhs_start);
        if !set_maparg_lhs_rhs(
            lhs_to_replace.as_ptr().cast(),
            orig_lhs_len,
            rhs_start,
            orig_rhs_len,
            LUA_NOREF,
            opt_vars().p_cpo,
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
    keys: impl crate::strings::IntoCCharPtr,
    args: &MapArguments,
    noremap: i32,
    mode: i32,
    is_abbr: bool,
    sid: Scid,
    lnum: Linenr,
    simplified: bool,
    alt: Option<std::rc::Rc<MapAltLink>>,
) {
    let keys = keys.into_c_char_ptr();
    {
        if crate::memory::cstr_first_byte(keys) == CTRL_C_CONST {
            if local {
                buf.b_mapped_ctrl_c |= mode;
            } else {
                g_st().mapped_ctrl_c |= mode;
            }
        }

        let m_keys = crate::memory::cstr_to_cstring(keys);
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
                map_st().first_abbr.insert(0, mp);
            }
        } else {
            let n = map_hash(
                mp.m_mode,
                crate::memory::cstr_first_byte(mp.m_keys.as_ptr()),
            );
            if local {
                (&mut buf.b_maphash)[n].insert(0, mp);
            } else {
                map_st().maphash[n].insert(0, mp);
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
    unsafe {
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

        let mut lhs = args.lhs.as_ptr();
        let did_simplify = args.alt_lhs_len != 0;

        'outer: {
            for keyround in 1..=2 {
                let mut did_it = false;
                let mut did_local = false;
                let keyround1_simplified = keyround == 1 && did_simplify;
                let mut len = args.lhs_len as i32;

                if keyround == 2 {
                    if !did_simplify {
                        break;
                    }
                    lhs = args.alt_lhs.as_ptr();
                    len = args.alt_lhs_len as i32;
                } else if did_simplify && do_print {
                    lhs = args.alt_lhs.as_ptr();
                    len = args.alt_lhs_len as i32;
                }

                if has_lhs {
                    if (len as usize) > MAXMAPLEN {
                        retval = 1;
                        break 'outer;
                    }

                    if is_abbrev && maptype != MAPTYPE_UNMAP {
                        let mut same: i32 = -1;
                        let mut keys_unescaped = [0u8; MAXMAPLEN + 1];
                        crate::memory::cstr_ncpy(
                            keys_unescaped.as_mut_ptr().cast(),
                            lhs.cast(),
                            len as usize,
                        );
                        let keys_unescaped_len = vim_unescape_ks(&mut keys_unescaped);
                        let mut p = keys_unescaped.as_ptr();
                        let end_p = p.wrapping_add(keys_unescaped_len);

                        let first = vim_iswordp(p.cast());
                        let mut last = first;
                        p = p.wrapping_add(utfc_ptr2len(p.cast()) as usize);
                        let mut n: i32 = 1;
                        while (p as usize) < (end_p as usize) {
                            n += 1;
                            last = vim_iswordp(p.cast());
                            if same == -1 && last != first {
                                same = n - 1;
                            }
                            p = p.wrapping_add(utfc_ptr2len(p.cast()) as usize);
                        }
                        if last && n > 2 && same >= 0 && same < n - 1 {
                            retval = 1;
                            break 'outer;
                        }

                        for i in 0..(len as usize) {
                            if ascii_iswhite(crate::memory::cstr_byte_at(lhs.cast(), i) as i32) {
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
                    let lhs_slice = crate::memory::cstr_slice(lhs.cast(), len as usize);
                    for bucket in get_map_buckets(ptr::null(), is_abbrev) {
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
                                    retval = 6;
                                    break 'outer;
                                }
                            }
                        }
                    }
                }

                if !local && !has_rhs && maptype != MAPTYPE_UNMAP {
                    for bucket in get_map_buckets(&raw mut *buf, is_abbrev) {
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
                                    let lhs_slice = crate::memory::cstr_slice(lhs.cast(), cmp_len);
                                    if mp_slice == lhs_slice {
                                        showmap(mp, true);
                                        did_local = true;
                                    }
                                }
                            }
                        }
                    }
                }

                let num_rounds = if maptype == MAPTYPE_UNMAP && !unmap_lhs_only {
                    2
                } else {
                    1
                };
                for round in 0..num_rounds {
                    if did_it || g_st().got_int {
                        break;
                    }
                    let (hash_start, hash_end) = if (round == 0 && has_lhs) || is_abbrev {
                        let start = if is_abbrev {
                            0
                        } else {
                            map_hash(mode, crate::memory::cstr_first_byte(lhs.cast()))
                        };
                        (start, start + 1)
                    } else {
                        (0, 256)
                    };
                    for hash in hash_start..hash_end {
                        if g_st().got_int {
                            break;
                        }
                        let mpp: &mut Vec<Mapblock> = if is_abbrev {
                            if local {
                                &mut buf.b_first_abbr
                            } else {
                                &mut map_st().first_abbr
                            }
                        } else if local {
                            let bmh = &raw mut buf.b_maphash;
                            &mut (*bmh)[hash]
                        } else {
                            &mut map_st().maphash[hash]
                        };
                        let mut idx: usize = 0;
                        while idx < mpp.len() && !g_st().got_int {
                            let mp: &mut Mapblock = &mut mpp[idx];
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
                                let (n, p) = if round != 0 {
                                    (mp.m_str.as_bytes().len() as i32, mp.m_str.as_ptr())
                                } else {
                                    (mp.m_keylen, mp.m_keys.as_ptr())
                                };
                                let cmp_len = n.min(len) as usize;
                                let p_slice = crate::memory::cstr_slice(p, cmp_len);
                                let lhs_slice = crate::memory::cstr_slice(lhs.cast(), cmp_len);
                                if p_slice == lhs_slice {
                                    if maptype == MAPTYPE_UNMAP {
                                        if n != len
                                            && (!is_abbrev
                                                || round != 0
                                                || n > len
                                                || !crate::memory::cstr_is_empty(skipwhite(
                                                    lhs.wrapping_add(n as usize).cast(),
                                                )))
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
                                        retval = 5;
                                        break 'outer;
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
                                        mpp.remove(idx);
                                        continue;
                                    }

                                    let new_hash = map_hash(
                                        mp.m_mode,
                                        crate::memory::cstr_first_byte(mp.m_keys.as_ptr()),
                                    );
                                    if !is_abbrev && new_hash != hash {
                                        let moved = mpp.remove(idx);
                                        if local {
                                            let bmh = &raw mut buf.b_maphash;
                                            (*bmh)[new_hash].insert(0, moved);
                                        } else {
                                            map_st().maphash[new_hash].insert(0, moved);
                                        }
                                        continue;
                                    }
                                }
                            }
                            idx += 1;
                        }
                    }
                }

                if maptype == MAPTYPE_UNMAP {
                    if !did_it {
                        if !keyround1_simplified {
                            retval = 2;
                        }
                    } else if *lhs == CTRL_C_CONST {
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
                    lhs.cast(),
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
}

/// Set or remove a mapping or an abbreviation in the current buffer, OR
/// display (matching) mappings/abbreviations.
pub fn do_map(
    maptype: i32,
    arg: impl crate::strings::IntoCCharPtr,
    mode: i32,
    is_abbrev: bool,
) -> i32 {
    let arg = arg.into_c_char_ptr();
    {
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
}

/// Get the mapping mode from the command name.
fn get_map_mode<T>(cmdp: *mut *mut T, forceit: bool) -> i32 {
    let mut p = unsafe { (*cmdp).cast::<u8>() };
    let modec = crate::memory::cstr_first_byte(p.cast());
    p = p.wrapping_add(1);
    let mode = if modec == b'i' {
        MODE_INSERT
    } else if modec == b'l' {
        MODE_LANGMAP
    } else if modec == b'c' {
        MODE_CMDLINE
    } else if modec == b'n' && crate::memory::cstr_first_byte(p.cast()) != b'o' {
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
        p = p.wrapping_sub(1);
        if forceit {
            MODE_INSERT | MODE_CMDLINE
        } else {
            MODE_VISUAL | MODE_SELECT | MODE_NORMAL | MODE_OP_PENDING
        }
    };

    unsafe {
        *cmdp = p.cast();
    }
    mode
}

fn do_mapclear(
    cmdp: impl crate::strings::IntoCCharPtr,
    arg: impl crate::strings::IntoCCharPtr,
    forceit: i32,
    abbr: i32,
) {
    let mut cmdp = cmdp.into_c_char_ptr().cast_mut();
    let arg = arg.into_c_char_ptr();
    {
        let local = crate::memory::cstr_as_bytes(arg) == b"<buffer>";
        if !local && !crate::memory::cstr_is_empty(arg) {
            emsg(gettext(E_INVARG));
            return;
        }

        let mode = get_map_mode(&mut cmdp, forceit != 0);
        map_clear_mode(curbuf_mut(), mode, local, abbr != 0);
    }
}

/// Clear all mappings in "mode".
pub fn map_clear_mode(buf: &mut FileBuf, mode: i32, local: bool, abbr: bool) {
    unsafe {
        for hash in 0..256 {
            let mpp: &mut Vec<Mapblock> = if abbr {
                if hash > 0 {
                    break;
                }
                if local {
                    &mut buf.b_first_abbr
                } else {
                    &mut map_st().first_abbr
                }
            } else if local {
                let mh = &raw mut buf.b_maphash;
                &mut (*mh)[hash]
            } else {
                &mut map_st().maphash[hash]
            };
            let mut idx: usize = 0;
            while idx < mpp.len() {
                let mp: &mut Mapblock = &mut mpp[idx];
                if (mp.m_mode & mode) != 0 {
                    mp.m_mode &= !mode;
                    if mp.m_mode == 0 {
                        mpp.remove(idx);
                        continue;
                    }
                    let new_hash = map_hash(
                        mp.m_mode,
                        crate::memory::cstr_first_byte(mp.m_keys.as_ptr()),
                    );
                    if !abbr && new_hash != hash {
                        let moved = mpp.remove(idx);
                        if local {
                            (&mut buf.b_maphash)[new_hash].insert(0, moved);
                        } else {
                            map_st().maphash[new_hash].insert(0, moved);
                        }
                        continue;
                    }
                }
                idx += 1;
            }
        }
    }
}

/// Check if a map exists that has given string in the rhs.
pub fn map_to_exists(
    str_ptr: impl crate::strings::IntoCCharPtr,
    modechars: impl crate::strings::IntoCCharPtr,
    abbr: bool,
) -> bool {
    let str_ptr = str_ptr.into_c_char_ptr();
    let modechars = modechars.into_c_char_ptr();
    {
        let mut mode: i32 = 0;
        let rhs = replace_termcodes(
            str_ptr,
            crate::memory::cstr_len(str_ptr),
            false,
            0,
            REPTERM_DO_LT,
            None,
            opt_vars().p_cpo,
        )
        .unwrap_or_default();

        let mode_bytes = crate::memory::cstr_as_bytes(modechars);
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

        map_to_exists_mode(rhs.as_ptr(), mode, abbr)
    }
}

/// Check if a map exists that has given string in the rhs.
pub fn map_to_exists_mode(rhs: impl crate::strings::IntoCCharPtr, mode: i32, abbr: bool) -> bool {
    let rhs = rhs.into_c_char_ptr();
    {
        for exp_buffer in [false, true] {
            let b = if exp_buffer {
                g_st().curbuf
            } else {
                ptr::null()
            };
            for bucket in get_map_buckets(b, abbr) {
                for mp in bucket {
                    let rhs_b = crate::memory::cstr_as_bytes(rhs);
                    if (mp.m_mode & mode) != 0
                        && (rhs_b.is_empty()
                            || mp.m_str.as_bytes().windows(rhs_b.len()).any(|w| w == rhs_b))
                    {
                        return true;
                    }
                }
            }
        }

        false
    }
}

fn translate_mapping(
    str_in: impl crate::strings::IntoCCharPtr,
    cpo_val: impl crate::strings::IntoCCharPtr,
) -> CString {
    let str_in = str_in.into_c_char_ptr();
    let cpo_val = cpo_val.into_c_char_ptr();
    {
        let mut str_ptr = str_in.cast::<u8>();
        let mut buf: Vec<u8> = Vec::with_capacity(40);

        let cpo_bslash = crate::strings::vim_haschr(cpo_val, K_CPO_BSLASH_CONST);

        while unsafe { *str_ptr } != 0 {
            let mut c = unsafe { *str_ptr } as i32;
            if c == K_SPECIAL as i32
                && unsafe { *str_ptr.wrapping_add(1) } != 0
                && unsafe { *str_ptr.wrapping_add(2) } != 0
            {
                let mut modifiers: i32 = 0;
                if unsafe { *str_ptr.wrapping_add(1) } == KS_MODIFIER {
                    str_ptr = str_ptr.wrapping_add(1);
                    str_ptr = str_ptr.wrapping_add(1);
                    modifiers = unsafe { *str_ptr } as i32;
                    str_ptr = str_ptr.wrapping_add(1);
                    c = unsafe { *str_ptr } as i32;
                }

                if c == K_SPECIAL as i32
                    && unsafe { *str_ptr.wrapping_add(1) } != 0
                    && unsafe { *str_ptr.wrapping_add(2) } != 0
                {
                    c = to_special(
                        i32::from(crate::memory::cstr_byte_at(str_ptr.cast(), 1)),
                        i32::from(crate::memory::cstr_byte_at(str_ptr.cast(), 2)),
                    );
                    if c == K_ZERO {
                        c = NUL as i32;
                    }
                    str_ptr = str_ptr.wrapping_add(2);
                }
                if is_special(c) || modifiers != 0 {
                    let s = get_special_key_name(c, modifiers);
                    buf.extend_from_slice(s.to_bytes());
                    str_ptr = str_ptr.wrapping_add(1);
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
            str_ptr = str_ptr.wrapping_add(1);
        }
        crate::memory::cstr_from_bytes(&buf)
    }
}

/// Work out what to complete when doing command line completion of mapping
/// or abbreviation names.
pub fn set_context_in_map_cmd<T>(
    xp: &mut ExpandCtx,
    cmd: impl crate::strings::IntoCCharPtr,
    arg: *mut T,
    forceit: bool,
    isabbrev: bool,
    isunmap: bool,
    cmdidx: crate::cmdexpand::Cmdidx,
) -> *mut T {
    let mut cmd = cmd.into_c_char_ptr().cast_mut();
    let mut arg = arg.cast::<u8>();
    {
        if forceit
            && cmdidx != crate::cmdexpand::Cmdidx::CmdMap
            && cmdidx != crate::cmdexpand::Cmdidx::CmdUnmap
        {
            xp.xp_context = EXPAND_NOTHING;
        } else {
            if isunmap {
                map_st().expand_mapmodes = get_map_mode(&mut cmd, forceit || isabbrev);
            } else {
                map_st().expand_mapmodes = MODE_INSERT | MODE_CMDLINE;
                if !isabbrev {
                    map_st().expand_mapmodes |=
                        MODE_VISUAL | MODE_SELECT | MODE_NORMAL | MODE_OP_PENDING;
                }
            }
            map_st().expand_isabbrev = isabbrev;
            xp.xp_context = EXPAND_MAPPINGS;
            map_st().expand_buffer = false;
            loop {
                let bytes = crate::memory::cstr_as_bytes(arg.cast());
                if bytes.starts_with(b"<buffer>") {
                    map_st().expand_buffer = true;
                    arg = skipwhite(arg.wrapping_add(8).cast()).cast();
                    continue;
                }
                if bytes.starts_with(b"<unique>") {
                    arg = skipwhite(arg.wrapping_add(8).cast()).cast();
                    continue;
                }
                if bytes.starts_with(b"<nowait>") {
                    arg = skipwhite(arg.wrapping_add(8).cast()).cast();
                    continue;
                }
                if bytes.starts_with(b"<silent>") {
                    arg = skipwhite(arg.wrapping_add(8).cast()).cast();
                    continue;
                }
                if bytes.starts_with(b"<special>") {
                    arg = skipwhite(arg.wrapping_add(9).cast()).cast();
                    continue;
                }
                if bytes.starts_with(b"<script>") {
                    arg = skipwhite(arg.wrapping_add(8).cast()).cast();
                    continue;
                }
                if bytes.starts_with(b"<expr>") {
                    arg = skipwhite(arg.wrapping_add(6).cast()).cast();
                    continue;
                }
                break;
            }
            xp.xp_pattern = arg.cast();
        }

        ptr::null_mut()
    }
}

/// Find all mapping/abbreviation names that match regexp "regmatch".
pub fn expand_mappings(
    pat: impl crate::strings::IntoCCharPtr,
    regmatch: &mut Regmatch,
    matches: &mut Vec<CString>,
) -> bool {
    let pat = pat.into_c_char_ptr();
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
            4 if !map_st().expand_buffer => c"<buffer>",
            5 => c"<nowait>",
            6 => c"<special>",
            _ => continue,
        };
        let p = p_cstr.as_ptr();

        let mut score: i32 = 0;
        let match_found = if !fuzzy {
            vim_regexec(regmatch, p, 0)
        } else {
            score = fuzzy_match_str(p, pat);
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

    let exp_buf = if !map_st().expand_isabbrev && map_st().expand_buffer {
        g_st().curbuf
    } else {
        ptr::null()
    };
    for bucket in get_map_buckets(exp_buf, map_st().expand_isabbrev) {
        for mp in bucket {
            if mp.m_simplified != 0 || (mp.m_mode & map_st().expand_mapmodes) == 0 {
                continue;
            }

            let p_cstr = translate_mapping(mp.m_keys.as_ptr().cast_mut(), opt_vars().p_cpo);
            let p = p_cstr.as_ptr().cast_mut();

            let mut score: i32 = 0;
            let match_found = if !fuzzy {
                vim_regexec(regmatch, p, 0)
            } else {
                score = fuzzy_match_str(p, pat);
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
pub fn check_abbr<T>(mut c: i32, ptr_arg: *mut T, col: i32, mincol: i32) -> bool {
    let mut ptr_arg = ptr_arg.cast::<u8>();
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
        let mut is_id = true;
        let mut p: *mut u8 = mb_prevptr(ptr_arg, ptr_arg.wrapping_add(col as usize));
        let vim_abbr = if !vim_iswordp(p.cast()) {
            true
        } else {
            if (p as usize) > (ptr_arg as usize) {
                is_id = vim_iswordp(mb_prevptr(ptr_arg, p).cast());
            }
            false
        };
        clen = 1;
        let min_ptr = ptr_arg.wrapping_add(mincol.max(0) as usize);
        while (p as usize) > (min_ptr as usize) {
            p = mb_prevptr(ptr_arg, p);
            if ascii_isspace(crate::memory::cstr_first_byte(p.cast()) as i32)
                || (!vim_abbr && is_id != vim_iswordp(p.cast()))
            {
                p = p.wrapping_add(utfc_ptr2len(p.cast()) as usize);
                break;
            }
            clen += 1;
        }
        scol = ((p as isize) - (ptr_arg as isize)) as i32;
    }

    if scol < mincol {
        scol = mincol;
    }
    if scol < col {
        ptr_arg = ptr_arg.wrapping_add(scol as usize);
        let mut len = col - scol;
        let mut mp_opt: Option<&Mapblock> = None;
        let first_abbr_slice = map_st().first_abbr.as_slice();
        for cand in curbuf_mut().b_first_abbr.iter().chain(first_abbr_slice) {
            let mut qlen = cand.m_keylen;
            let mut q_vec: Vec<u8>;
            let mut q: *mut u8 = cand.m_keys.as_ptr().cast_mut().cast();

            if cand.m_keys.as_bytes().contains(&K_SPECIAL) {
                q_vec = cand.m_keys.as_bytes_with_nul().to_vec();
                qlen = vim_unescape_ks(&mut q_vec) as i32;
                q = q_vec.as_mut_ptr();
            }
            let match_found = (cand.m_mode & g_st().state_global) != 0
                && qlen == len
                && crate::memory::cstr_slice(q.cast(), len as usize)
                    == crate::memory::cstr_slice(ptr_arg.cast(), len as usize);
            if match_found {
                mp_opt = Some(cand);
                break;
            }
        }
        if let Some(mp) = mp_opt {
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
                    let mut newlen =
                        utf_char2bytes(c, tb.as_mut_ptr().wrapping_add(j).cast()) as usize;
                    tb[j + newlen] = 0;
                    let escaped = vim_strsave_escape_ks(tb.as_mut_ptr().wrapping_add(j).cast());
                    let escaped_bytes = escaped.to_bytes();
                    newlen = escaped_bytes.len();
                    unsafe {
                        ptr::copy(
                            escaped_bytes.as_ptr(),
                            tb.as_mut_ptr().wrapping_add(j),
                            newlen,
                        )
                    };
                    j += newlen;
                }
                tb[j] = 0;
                ins_typebuf(tb.as_mut_ptr().cast(), 1, 0, true, mp.m_silent != 0);
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
                let s = cs.as_ptr().cast_mut();
                ins_typebuf(s, noremap, 0, true, silent);
                g_st().typebuf.tb_no_abbr_cnt += (cs.to_bytes().len() + j + 1) as i32;
            }

            tb[0] = CTRL_H_CONST;
            tb[1] = 0;
            len = clen;
            while len > 0 {
                len -= 1;
                ins_typebuf(tb.as_mut_ptr().cast(), 1, 0, true, silent);
            }
            return true;
        }
    }
    false
}

/// Evaluate the RHS of a mapping or abbreviations and take care of escaping
/// special characters.
pub fn eval_map_expr(mp: &Mapblock, c: i32) -> Option<CString> {
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
        let args = Array {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        let ret = nlua_call_ref(
            luaref,
            ptr::null(),
            args,
            K_RET_OBJECT,
            None,
            Some(&mut err),
        );
        if ret.obj_type == ObjectType::ObjectTypeString {
            p = Some(string_to_cstr(ret.data.string()));
        }
        api_free_object(ret);
        if err.err_type != ErrorType::ErrorTypeNone {
            semsg_multiline!(c"emsg", c"E5108: %s", err.msg_cstr());
            api_clear_error(&mut err);
        }
    } else {
        p = eval_to_string(expr_bytes.as_mut_ptr().cast(), false, false);
    }
    g_st().expr_map_lock -= 1;
    curwin_mut().w_cursor = save_cursor;
    g_st().msg_col = save_msg_col;
    g_st().msg_row = save_msg_row;

    let p = p?;

    if replace_keycodes {
        replace_termcodes(
            p.as_ptr(),
            p.to_bytes().len(),
            false,
            0,
            REPTERM_DO_LT,
            None,
            opt_vars().p_cpo,
        )
    } else {
        Some(vim_strsave_escape_ks(p.as_ptr()))
    }
}

/// Write map commands for the current mappings to an .exrc file.
pub fn makemap(fd: *mut FILE, buf: *mut FileBuf) -> bool {
    let mut did_cpo = false;

    for abbr in 0..2 {
        for bucket in get_map_buckets(buf, abbr != 0) {
            for mp in bucket {
                if mp.m_simplified != 0 || mp.m_noremap == REMAP_SCRIPT || mp.m_luaref != LUA_NOREF
                {
                    continue;
                }

                let mut p = mp.m_str.as_ptr();
                while !crate::memory::cstr_is_empty(p) {
                    if crate::memory::cstr_first_byte(p) == K_SPECIAL
                        && crate::memory::cstr_byte_at(p, 1) == KS_EXTRA
                        && crate::memory::cstr_byte_at(p, 2) == KE_SNR
                    {
                        break;
                    }
                    p = p.wrapping_add(1);
                }
                if !crate::memory::cstr_is_empty(p) {
                    continue;
                }

                let mut c1: u8 = 0;
                let mut c2: u8 = 0;
                let mut c3: u8 = 0;
                let mut cmd: &CStr = if abbr != 0 { c"abbr" } else { c"map" };

                const MODE_NVSO: i32 = MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING;
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
                            && (fputs(c"let s:cpo_save=&cpo", fd) < 0
                                || !put_eol(fd)
                                || fputs(c"set cpo&vim", fd) < 0
                                || !put_eol(fd))
                        {
                            return false;
                        }
                    }
                    if c1 != 0 && unsafe { fputc(c1 as i32, fd) } < 0 {
                        return false;
                    }
                    if mp.m_noremap != REMAP_YES && fputs(c"nore", fd) < 0 {
                        return false;
                    }
                    if fputs(cmd, fd) < 0 {
                        return false;
                    }
                    if !buf.is_null() && fputs(c" <buffer>", fd) < 0 {
                        return false;
                    }
                    if mp.m_nowait != 0 && fputs(c" <nowait>", fd) < 0 {
                        return false;
                    }
                    if mp.m_silent != 0 && fputs(c" <silent>", fd) < 0 {
                        return false;
                    }
                    if mp.m_expr != 0 && fputs(c" <expr>", fd) < 0 {
                        return false;
                    }

                    if unsafe { fputc(b' ' as i32, fd) } < 0
                        || !put_escstr(fd, mp.m_keys.as_ptr(), 0)
                        || unsafe { fputc(b' ' as i32, fd) } < 0
                        || !put_escstr(fd, mp.m_str.as_ptr(), 1)
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
    }
    if did_cpo
        && (fputs(c"let &cpo=s:cpo_save", fd) < 0
            || !put_eol(fd)
            || fputs(c"unlet s:cpo_save", fd) < 0
            || !put_eol(fd))
    {
        return false;
    }
    true
}

/// Write escape string to file.
pub fn put_escstr(fd: *mut FILE, strstart: impl crate::strings::IntoCCharPtr, what: i32) -> bool {
    let strstart = strstart.into_c_char_ptr();
    {
        let mut str_ptr = strstart.cast::<u8>();

        if unsafe { *str_ptr } == 0 && what == 1 {
            if fputs(c"<Nop>", fd) < 0 {
                return false;
            }
            return true;
        }

        while unsafe { *str_ptr } != 0 {
            if let Some(p) = mb_unescape(&mut str_ptr) {
                for &p_byte in p.to_bytes() {
                    if unsafe { fputc(p_byte as i32, fd) } < 0 {
                        return false;
                    }
                }
                continue;
            }

            let mut c = unsafe { *str_ptr } as i32;
            if c == K_SPECIAL as i32 && what != 2 {
                let mut modifiers: i32 = 0;
                if unsafe { *str_ptr.wrapping_add(1) } == KS_MODIFIER {
                    modifiers = i32::from(crate::memory::cstr_byte_at(str_ptr.cast(), 2));
                    str_ptr = str_ptr.wrapping_add(3);

                    if let Some(p) = mb_unescape(&mut str_ptr) {
                        c = utf_ptr2char(p);
                        str_ptr = str_ptr.wrapping_sub(1);
                    } else {
                        c = unsafe { *str_ptr } as i32;
                    }
                }
                if c == K_SPECIAL as i32 {
                    c = to_special(
                        i32::from(crate::memory::cstr_byte_at(str_ptr.cast(), 1)),
                        i32::from(crate::memory::cstr_byte_at(str_ptr.cast(), 2)),
                    );
                    str_ptr = str_ptr.wrapping_add(2);
                }
                if is_special(c) || modifiers != 0 {
                    if fputs(get_special_key_name(c, modifiers).as_ptr(), fd) < 0 {
                        return false;
                    }
                    str_ptr = str_ptr.wrapping_add(1);
                    continue;
                }
            }

            if c == NL {
                if what == 2 {
                    if fputs(c"\\\x16\n", fd) < 0 {
                        return false;
                    }
                } else if fputs(c"<NL>", fd) < 0 {
                    return false;
                }
                str_ptr = str_ptr.wrapping_add(1);
                continue;
            }

            if what == 2 && (ascii_iswhite(c) || c == b'"' as i32 || c == b'\\' as i32) {
                if unsafe { fputc(b'\\' as i32, fd) } < 0 {
                    return false;
                }
            } else if (c < b' ' as i32
                || c > b'~' as i32
                || c == b'|' as i32
                || (what == 0 && c == b' ' as i32)
                || (what == 1 && ptr::eq(str_ptr, strstart.cast::<u8>()) && c == b' ' as i32)
                || (what != 2 && c == b'<' as i32))
                && unsafe { fputc(CTRL_V_CONST, fd) } < 0
            {
                return false;
            }
            if unsafe { fputc(c, fd) } < 0 {
                return false;
            }
            str_ptr = str_ptr.wrapping_add(1);
        }
        true
    }
}

/// Check the string "keys" against the lhs of all mappings.
pub fn check_map(
    keys: impl crate::strings::IntoCCharPtr,
    mode: i32,
    exact: i32,
    ign_mod: i32,
    abbr: i32,
    mut mp_out: Option<&mut Option<&'static Mapblock>>,
    local_ptr: &mut i32,
    rhs_lua: &mut i32,
) -> Option<&'static CStr> {
    let keys = keys.into_c_char_ptr();
    {
        *rhs_lua = LUA_NOREF;

        let len = crate::memory::cstr_len(keys) as i32;
        for local in (0..=1).rev() {
            let b = if local != 0 {
                g_st().curbuf
            } else {
                ptr::null()
            };
            for bucket in get_map_buckets(b, abbr != 0) {
                for mp in bucket {
                    if (mp.m_mode & mode) != 0 && (exact == 0 || mp.m_keylen == len) {
                        let mut s = mp.m_keys.as_ptr();
                        let mut keylen = mp.m_keylen;
                        if ign_mod != 0
                            && keylen >= 3
                            && crate::memory::cstr_first_byte(s) == K_SPECIAL
                            && crate::memory::cstr_byte_at(s, 1) == KS_MODIFIER
                        {
                            s = s.wrapping_add(3);
                            keylen -= 3;
                        }
                        let minlen = keylen.min(len).max(0) as usize;
                        if crate::memory::cstr_slice(s, minlen)
                            == crate::memory::cstr_slice(keys, minlen)
                        {
                            if let Some(ref mut out) = mp_out {
                                **out = Some(mp);
                            }
                            {
                                *local_ptr = local;
                            }
                            *rhs_lua = mp.m_luaref;
                            return if mp.m_luaref == LUA_NOREF {
                                Some(mp.m_str.as_c_str())
                            } else {
                                None
                            };
                        }
                    }
                }
            }
        }

        None
    }
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
        crate::eval::typval::TypvalVval::Number(i64::from(map_to_exists(name, mode, abbr)));
}

fn mapblock_fill_dict(
    mp: &Mapblock,
    lhsrawalt: impl crate::strings::IntoCCharPtr,
    buffer_value: i32,
    abbr: bool,
    compatible: bool,
    arena: &mut Arena,
) -> Dict {
    let lhsrawalt = lhsrawalt.into_c_char_ptr();
    let mut dict = arena_dict(arena, 20);
    let lhs = str2special_arena(
        mp.m_keys.as_ptr(),
        compatible,
        if compatible { K_FALSE } else { K_NONE },
        arena,
    );
    let mapmode = arena_alloc(arena, 7, false).cast::<u8>();
    map_mode_to_chars(mp.m_mode, mapmode);
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
    } else {
        let rhs = cstr_as_string(if compatible {
            mp.m_orig_str.as_c_str()
        } else {
            str2special_arena(mp.m_str.as_ptr(), false, K_NONE, arena)
        });
        put_c(&mut dict, c"rhs", string_obj(rhs));
    }
    if let Some(ref desc) = mp.m_desc {
        put_c(&mut dict, c"desc", cstr_as_obj(desc.as_ptr()));
    }
    put_c(&mut dict, c"lhs", cstr_as_obj(lhs));
    put_c(&mut dict, c"lhsraw", cstr_as_obj(mp.m_keys.as_ptr()));
    if !lhsrawalt.is_null() {
        let len = crate::memory::cstr_len(lhsrawalt);
        let alt_arena = arena_alloc(arena, len + 1, false);
        crate::memory::cstr_cpy(alt_arena.cast(), lhsrawalt);
        put_c(&mut dict, c"lhsrawalt", cstr_as_obj(alt_arena.cast()));
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
    put_c(&mut dict, c"mode", cstr_as_obj(mapmode.cast()));
    put_c(&mut dict, c"abbr", integer_obj(i64::from(abbr)));
    put_c(&mut dict, c"mode_bits", integer_obj(i64::from(mp.m_mode)));

    dict
}

fn get_maparg(argvars: &mut [TypVal], rettv: &mut TypVal, exact: i32) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

    let keys = tv_get_string(&argvars[0]);
    if crate::memory::cstr_is_empty(keys) {
        return;
    }

    let mut buf = [0u8; NUMBUFLEN];
    let mut abbr = false;
    let mut get_dict = false;

    let which = if argvars[1].v_type != VarType::VarUnknown {
        let w = tv_get_string_buf_chk(&argvars[1], &mut buf)
            .map_or(ptr::null(), std::ffi::CStr::as_ptr);
        if argvars[2].v_type != VarType::VarUnknown {
            abbr = tv_get_bool(&argvars[2]) != 0;
            if argvars[3].v_type != VarType::VarUnknown {
                get_dict = tv_get_bool(&argvars[3]) != 0;
            }
        }
        w
    } else {
        c"".as_ptr()
    };
    if which.is_null() {
        return;
    }

    let mut did_simplify = false;
    let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
    let mut which_mut = which.cast_mut();
    let mode = get_map_mode(&mut which_mut, false);

    let keys_len = crate::memory::cstr_len(keys);
    let keys_simplified = replace_termcodes(
        keys,
        keys_len,
        false,
        0,
        flags,
        Some(&mut did_simplify),
        opt_vars().p_cpo,
    )
    .unwrap_or_default();
    let mut mp_opt: Option<&'static Mapblock> = None;
    let mut buffer_local: i32 = 0;
    let mut rhs_lua: LuaRef = LUA_NOREF;
    let mut rhs = check_map(
        keys_simplified.as_ptr().cast_mut(),
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
            keys,
            keys_len,
            false,
            0,
            flags | REPTERM_NO_SIMPLIFY,
            None,
            opt_vars().p_cpo,
        )
        .unwrap_or_default();
        rhs = check_map(
            alt_keys.as_ptr().cast_mut(),
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
                crate::eval::typval::tv_set_string(&mut *rettv, c"<Nop>");
            } else {
                crate::eval::typval::tv_set_string(
                    &mut *rettv,
                    str2special_save(rhs.as_ptr(), false, K_FALSE).as_ptr(),
                );
            }
        } else if rhs_lua != LUA_NOREF
            && let Some(mp) = mp_opt
        {
            let s = nlua_funcref_str(mp.m_luaref, true);
            crate::eval::typval::tv_set_string_len(&mut *rettv, s.as_ptr(), s.as_bytes().len());
        }
    } else if let Some(mp) = mp_opt
        && (rhs.is_some() || rhs_lua != LUA_NOREF)
    {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let dict = mapblock_fill_dict(
            mp,
            if did_simplify {
                keys_simplified.as_ptr()
            } else {
                ptr::null()
            },
            buffer_local,
            abbr,
            true,
            &mut arena,
        );
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

fn get_map_mode_string(mode_string: impl crate::strings::IntoCCharPtr, abbr: bool) -> i32 {
    let mode_string = mode_string.into_c_char_ptr();
    {
        let mut p = mode_string;
        const MASK_V: i32 = MODE_VISUAL | MODE_SELECT;
        const MASK_MAP: i32 = MODE_VISUAL | MODE_SELECT | MODE_NORMAL | MODE_OP_PENDING;
        const MASK_BANG: i32 = MODE_INSERT | MODE_CMDLINE;

        if crate::memory::cstr_is_empty(p) {
            p = c" ".as_ptr();
        }
        let mut mode: i32 = 0;
        while !crate::memory::cstr_is_empty(p) {
            let modec = crate::memory::cstr_first_byte(p);
            p = p.wrapping_add(1);
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
pub fn f_mapset(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        if check_secure() {
            return;
        }

        let mut buf = [0u8; NUMBUFLEN];
        let dict_only = argvars[0].v_type == VarType::VarDict;

        let (which, is_abbr, d) = if dict_only {
            let d = argvars[0].vval.v_dict();
            let which = tv_dict_get_string(d.as_ref(), c"mode");
            let is_abbr = tv_dict_get_bool(d.as_ref(), c"abbr", -1) as i32;
            let Some(which) = which else {
                emsg(gettext(E_ENTRIES_MISSING_IN_MAPSET_DICT_ARGUMENT));
                return;
            };
            if is_abbr < 0 {
                emsg(gettext(E_ENTRIES_MISSING_IN_MAPSET_DICT_ARGUMENT));
                return;
            }
            (which.as_ptr(), is_abbr, d)
        } else {
            let Some(which) = tv_get_string_buf_chk(&argvars[0], &mut buf) else {
                return;
            };
            let is_abbr = tv_get_bool(&argvars[1]) as i32;
            if !tv_check_for_dict_arg(argvars, 2) {
                return;
            }
            let d = argvars[2].vval.v_dict();
            (which.as_ptr(), is_abbr, d)
        };
        let mode = get_map_mode_string(which, is_abbr != 0);
        if mode == 0 {
            semsg!(gettext(E_ILLEGAL_MAP_MODE_STRING_STR), which);
            return;
        }

        let lhs = crate::eval::typval::tv_dict_get_cstring(d.as_ref(), c"lhs");
        let lhsraw = crate::eval::typval::tv_dict_get_cstring(d.as_ref(), c"lhsraw");
        let lhsrawalt = crate::eval::typval::tv_dict_get_cstring(d.as_ref(), c"lhsrawalt");
        let mut orig_rhs = crate::eval::typval::tv_dict_get_cstring(d.as_ref(), c"rhs");
        let mut rhs_lua: LuaRef = LUA_NOREF;
        let callback_di = tv_dict_find(d.as_ref(), c"callback", 8);
        if !callback_di.is_null() && (*callback_di).di_tv.v_type == VarType::VarFunc {
            let fp = find_func((*callback_di).di_tv.vval.v_string());
            if !fp.is_null() && ((*fp).uf_flags & FC_LUAREF) != 0 {
                rhs_lua = api_new_luaref((*fp).uf_luaref);
                orig_rhs = Some(std::ffi::CString::default());
            }
        }
        let (Some(lhs), Some(lhsraw), Some(orig_rhs)) = (lhs, lhsraw, orig_rhs) else {
            emsg(gettext(E_ENTRIES_MISSING_IN_MAPSET_DICT_ARGUMENT));
            api_free_luaref(rhs_lua);
            return;
        };

        let mut noremap = if tv_dict_get_number(d.as_ref(), c"noremap") != 0 {
            REMAP_NONE
        } else {
            0
        };
        if tv_dict_get_number(d.as_ref(), c"script") != 0 {
            noremap = REMAP_SCRIPT;
        }
        let mut args = MapArguments::new();
        args.expr = tv_dict_get_number(d.as_ref(), c"expr") != 0;
        args.silent = tv_dict_get_number(d.as_ref(), c"silent") != 0;
        args.nowait = tv_dict_get_number(d.as_ref(), c"nowait") != 0;
        args.replace_keycodes = tv_dict_get_number(d.as_ref(), c"replace_keycodes") != 0;
        args.desc = crate::eval::typval::tv_dict_get_cstring(d.as_ref(), c"desc");

        let sid = tv_dict_get_number(d.as_ref(), c"sid") as Scid;
        let lnum = tv_dict_get_number(d.as_ref(), c"lnum") as Linenr;
        let buffer = tv_dict_get_number(d.as_ref(), c"buffer") != 0;

        set_maparg_rhs(
            orig_rhs.as_c_str(),
            orig_rhs.as_bytes().len(),
            rhs_lua,
            sid,
            opt_vars().p_cpo,
            &mut args,
        );

        let mut unmap_args = MapArguments::new();
        set_maparg_lhs_rhs(
            lhs.as_c_str(),
            lhs.as_bytes().len(),
            c"",
            0,
            LUA_NOREF,
            opt_vars().p_cpo,
            &mut unmap_args,
        );
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
}

/// "maplist()" function
pub fn f_maplist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
    let abbr = argvars[0].v_type != VarType::VarUnknown && tv_get_bool(&argvars[0]) != 0;

    tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN_CONST);

    for buffer_local in 0..=1 {
        let b = if buffer_local != 0 {
            g_st().curbuf
        } else {
            ptr::null()
        };
        for bucket in get_map_buckets(b, abbr) {
            for mp in bucket {
                if mp.m_simplified != 0 {
                    continue;
                }

                let mut did_simplify = false;

                let mut arena = Arena {
                    cur_blk: ptr::null_mut(),
                    pos: 0,
                    size: 0,
                };
                let lhs = str2special_arena(mp.m_keys.as_ptr(), true, K_FALSE, &mut arena);
                let keys_buf = replace_termcodes(
                    lhs,
                    crate::memory::cstr_len(lhs),
                    false,
                    0,
                    flags,
                    Some(&mut did_simplify),
                    opt_vars().p_cpo,
                )
                .unwrap_or_default();

                let dict = mapblock_fill_dict(
                    mp,
                    if did_simplify {
                        keys_buf.as_ptr()
                    } else {
                        ptr::null()
                    },
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
                tv_list_append_dict(rettv.vval.v_list_ref(), d.vval.v_dict());
                arena_mem_free(arena_finish(&mut arena));
            }
        }
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
        lhs.as_ptr().cast_mut(),
        lhs.to_bytes().len(),
        rhs.as_ptr().cast_mut(),
        rhs.to_bytes().len(),
        LUA_NOREF,
        opt_vars().p_cpo,
        &mut args,
    );
    args.buffer = buffer;

    buf_do_map(MAPTYPE_NOREMAP, &mut args, mode, false, curbuf_mut());
}

fn langmap_set_entry(from: i32, to: i32) {
    let entries = &mut map_st().langmap_mapga;
    match entries.binary_search_by_key(&from, |e| e.from) {
        Ok(idx) => entries[idx].to = to,
        Err(idx) => entries.insert(idx, LangmapEntry { from, to }),
    }
}

/// Apply 'langmap' to multi-byte character "c" and return the result.
pub fn langmap_adjust_mb(c: i32) -> i32 {
    let entries = map_st().langmap_mapga.as_slice();
    entries
        .binary_search_by_key(&c, |e| e.from)
        .map_or(c, |idx| entries[idx].to)
}

pub fn langmap_init() {
    for i in 0..256 {
        unsafe {
            *(&raw mut g_st().langmap_mapchar)
                .cast::<u8>()
                .wrapping_add(i) = i as u8;
        }
    }
    map_st().langmap_mapga = Vec::new();
}

/// Called when langmap option is set; the language map can be
/// changed at any time!
pub fn did_set_langmap(args: &mut Optset) -> Option<&'static CStr> {
    langmap_init();

    let mut p = opt_vars().p_langmap;
    while !crate::memory::cstr_is_empty(p) {
        let mut p2 = p;
        while !crate::memory::cstr_is_empty(p2)
            && crate::memory::cstr_first_byte(p2) != b','
            && crate::memory::cstr_first_byte(p2) != b';'
        {
            if crate::memory::cstr_first_byte(p2) == b'\\'
                && crate::memory::cstr_byte_at(p2, 1) != 0
            {
                p2 = p2.wrapping_add(1);
            }
            p2 = p2.wrapping_add(utfc_ptr2len(p2) as usize);
        }
        if crate::memory::cstr_first_byte(p2) == b';' {
            p2 = p2.wrapping_add(1);
        } else {
            p2 = ptr::null_mut();
        }
        while !crate::memory::cstr_is_empty(p) {
            if crate::memory::cstr_first_byte(p) == b',' {
                p = p.wrapping_add(1);
                break;
            }
            if crate::memory::cstr_first_byte(p) == b'\\' && crate::memory::cstr_byte_at(p, 1) != 0
            {
                p = p.wrapping_add(1);
            }
            let from = utf_ptr2char(p);
            let from_ptr = p;
            let mut to: i32 = NUL as i32;
            let mut to_ptr = c"".as_ptr();
            if p2.is_null() {
                p = p.wrapping_add(utfc_ptr2len(p) as usize);
                if crate::memory::cstr_first_byte(p) != b',' {
                    if crate::memory::cstr_first_byte(p) == b'\\' {
                        p = p.wrapping_add(1);
                    }
                    to_ptr = p;
                    to = utf_ptr2char(p);
                }
            } else if crate::memory::cstr_first_byte(p2) != b',' {
                if crate::memory::cstr_first_byte(p2) == b'\\' {
                    p2 = p2.wrapping_add(1);
                }
                to_ptr = p2;
                to = utf_ptr2char(p2);
            }
            if to == NUL as i32 {
                return opt_error(
                    args.os_errbuf,
                    c"E357: 'langmap': Matching character missing for %s",
                    transchar(from),
                );
            }

            if from >= 256 {
                langmap_set_entry(from, to);
            } else {
                if to > UCHAR_MAX {
                    swmsg!(
                        true,
                        c"'langmap': Mapping from %.*s to %.*s will not work properly",
                        utf_ptr2len(from_ptr),
                        from_ptr,
                        utf_ptr2len(to_ptr),
                        to_ptr,
                    );
                }
                g_st().langmap_mapchar[(from & 255) as usize] = to as u8;
            }

            p = p.wrapping_add(utfc_ptr2len(p) as usize);
            if !p2.is_null() {
                p2 = p2.wrapping_add(utfc_ptr2len(p2) as usize);
                if crate::memory::cstr_first_byte(p) == b';' {
                    p = p2;
                    if !crate::memory::cstr_is_empty(p) {
                        if crate::memory::cstr_first_byte(p) != b',' {
                            return opt_error(
                                args.os_errbuf,
                                c"E358: 'langmap': Extra characters after semicolon: %s",
                                p,
                            );
                        }
                        p = p.wrapping_add(1);
                    }
                    break;
                }
            }
        }
    }

    None
}

fn do_exmap(eap: &mut Exarg, isabbrev: bool) {
    let mut cmdp = eap.cmd;
    let mode = get_map_mode(&mut cmdp, eap.forceit != 0 || isabbrev);

    let maptype = if crate::memory::cstr_first_byte(cmdp) == b'n' {
        MAPTYPE_NOREMAP
    } else if crate::memory::cstr_first_byte(cmdp) == b'u' {
        MAPTYPE_UNMAP
    } else {
        MAPTYPE_MAP
    };
    let mut parsed_args = MapArguments::new();
    let result = str_to_mapargs(eap.arg, maptype == MAPTYPE_UNMAP, &mut parsed_args);
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
                    parsed_args.lhs.as_ptr(),
                );
            }
            6 => {
                semsg!(
                    if isabbrev {
                        gettext(E_GLOBAL_ABBREVIATION_ALREADY_EXISTS_FOR_STR)
                    } else {
                        gettext(E_GLOBAL_MAPPING_ALREADY_EXISTS_FOR_STR)
                    },
                    parsed_args.lhs.as_ptr(),
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
        msg_outtrans(eap.cmd, 0, false);
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
    do_mapclear(eap.cmd, eap.arg, eap.forceit, 0);
}

/// ":abclear" and friends.
pub fn ex_abclear(eap: &mut Exarg) {
    do_mapclear(eap.cmd, eap.arg, 1, 1);
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
            parsed_args.desc = Some(string_to_cstr(opts.desc));
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
            lhs.data,
            lhs.size,
            rhs.data,
            rhs.size,
            lua_funcref,
            opt_vars().p_cpo,
            &mut parsed_args,
        ) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"LHS exceeds maximum map length: %s",
                lhs.data,
            );
            break 'block;
        }

        if parsed_args.lhs_len > MAXMAPLEN || parsed_args.alt_lhs_len > MAXMAPLEN {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"LHS exceeds maximum map length: %s",
                lhs.data,
            );
            break 'block;
        }

        let mut p: *mut u8 = if mode.size > 0 {
            mode.data.cast()
        } else {
            c"m".as_ptr().cast_mut().cast()
        };
        let forceit = crate::memory::cstr_first_byte(p.cast()) == b'!';
        let mode_val = get_map_mode(&mut p, forceit);
        if forceit {
            debug_assert!(ptr::eq(p, mode.data.cast()));
            p = p.wrapping_add(1);
        }
        let is_abbrev = (mode_val & (MODE_INSERT | MODE_CMDLINE)) != 0
            && crate::memory::cstr_first_byte(p.cast()) == b'a';
        if is_abbrev {
            p = p.wrapping_add(1);
        }
        if mode.size > 0 && ((p as usize) - (mode.data as usize)) != mode.size {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Invalid mode shortname: \"%s\"",
                mode.data,
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
                    parsed_args
                        .rhs
                        .as_ref()
                        .map_or(c"".as_ptr(), |s| s.as_ptr()),
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
                    lhs.data,
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
                    lhs.data,
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
pub fn keymap_array(mode: NvimString, buf: *mut FileBuf, arena: &mut Arena) -> Array {
    let mut mappings = ArrayBuilder::new();

    let mut p: *mut u8 = if mode.size > 0 {
        mode.data.cast()
    } else {
        c"m".as_ptr().cast_mut().cast()
    };
    let forceit = crate::memory::cstr_first_byte(p.cast()) == b'!';
    let int_mode = get_map_mode(&mut p, forceit);
    if forceit {
        debug_assert!(ptr::eq(p, mode.data.cast()));
        p = p.wrapping_add(1);
    }
    let is_abbrev = (int_mode & (MODE_INSERT | MODE_CMDLINE)) != 0
        && crate::memory::cstr_first_byte(p.cast()) == b'a';

    let buffer_value = if buf.is_null() {
        0
    } else {
        unsafe { (*buf).b_fnum }
    };

    for bucket in get_map_buckets(buf, is_abbrev) {
        for current_maphash in bucket {
            if current_maphash.m_simplified == 0 && (int_mode & current_maphash.m_mode) != 0 {
                let alt_cstr = current_maphash.alt_keys();
                let alt_keys = alt_cstr.as_ref().map_or(ptr::null(), |s| s.as_ptr());
                mappings.push(dict_obj(mapblock_fill_dict(
                    current_maphash,
                    alt_keys,
                    buffer_value,
                    is_abbrev,
                    false,
                    arena,
                )));
            }
        }
    }

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
            map_mode_to_chars(MODE_NORMAL | MODE_VISUAL, buf.as_mut_ptr());
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr().cast()), b"nx");

            langmap_init();
            assert_eq!(langmap_adjust_mb(b'a' as i32), b'a' as i32);
        }
    }
}
