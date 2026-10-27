//! Code for mappings and abbreviations (`src/nvim/mapping.c`).

use crate::api::buffer::KeyDict_keymap;
use crate::api::private::converter::object_to_vim_take_luaref;
use crate::api::private::helpers::{
    ArrayBuilder, api_clear_error, api_free_object, api_set_error, api_set_sctx, arena_dict,
    arena_take_arraybuilder, cstr_as_obj, cstr_as_string, dict_obj, find_buffer_by_handle, has_key,
    integer_obj, luaref_obj, put_c, string_obj, string_to_cstr,
};
use crate::api::private::validate::{Array, Dict, Error, ErrorType, NvimString, ObjectType};
use crate::api::tabpage::Buffer;
use crate::buffer::buf_T;
use crate::charset::{
    NUL, ascii_isspace, ascii_iswhite, is_special, skipwhite, to_special, transchar, vim_iswordp,
};
use crate::cmdexpand::{EXPAND_MAPPINGS, EXPAND_NOTHING, MB_MAXBYTES, cmdline_fuzzy_complete};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, TypvalVval, VAR_UNLOCKED, VarType, nlua_clear_ref, tv_check_for_dict_arg,
    tv_dict_alloc_ret, tv_dict_find, tv_dict_get_bool, tv_dict_get_number, tv_dict_get_string,
    tv_get_bool, tv_get_string, tv_get_string_buf, tv_get_string_buf_chk, tv_list_alloc_ret,
    tv_list_append_dict, typval_T,
};
use crate::eval::userfunc::{FC_LUAREF, find_func};
use crate::eval::vars::{scid_T, set_vim_var_char};
use crate::eval::{eval_to_string, kListLenUnknown, last_set_msg};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{Ctrl_C, exarg_T};
use crate::ex_session::put_eol;
use crate::fileio::{FILE, fputc, fputs};
use crate::fuzzy::{
    FUZZY_SCORE_NONE, fuzmatch_str_T, fuzzy_match_str, fuzzymatches_slice_to_strmatches,
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
    api_free_luaref, api_new_luaref, kRetObject, nlua_call_ref, nlua_funcref_str, nlua_set_sctx,
};
use crate::math::{FAIL, OK};
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
use crate::optionstr::optset_T;
use crate::os::env::expand_T;
use crate::regexp::{Ctrl_V, NUMBUFLEN, regmatch_T, vim_regexec};
use crate::runtime::{sctx_T, sourcing_lnum};
use crate::strings::{gettext, sort_strings, vim_snprintf, vim_strchr};
use crate::viml::parser::expressions::Ctrl_H;
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

use crate::cursor::linenr_T;

pub const MAXMAPLEN: usize = 50;
pub const MAX_MAPHASH: usize = 256;

pub(crate) const REMAP_YES: c_int = 0;
pub(crate) const REMAP_NONE: c_int = -1;

const UCHAR_MAX: c_int = 255;

pub const MAPTYPE_MAP: c_int = 0;
pub const MAPTYPE_UNMAP: c_int = 1;
pub const MAPTYPE_NOREMAP: c_int = 2;
pub const MAPTYPE_UNMAP_LHS: c_int = 3;

const kCpoBslash: c_int = b'B' as c_int;

const KEYSET_OPTIDX_keymap__desc: u64 = 1;
const KEYSET_OPTIDX_keymap__callback: u64 = 8;

pub struct MapAltLink {
    pub keys: [std::cell::Cell<Option<CString>>; 2],
}

#[repr(C)]
pub struct mapblock_T {
    pub m_alt: Option<std::rc::Rc<MapAltLink>>,
    pub m_keys: CString,
    pub m_str: CString,
    pub m_orig_str: CString,
    pub m_luaref: LuaRef,
    pub m_keylen: c_int,
    pub m_mode: c_int,
    pub m_simplified: c_int,
    pub m_noremap: c_int,
    pub m_silent: c_char,
    pub m_nowait: c_char,
    pub m_expr: c_char,
    pub m_script_ctx: sctx_T,
    pub m_desc: Option<CString>,
    pub m_replace_keycodes: bool,
}

impl mapblock_T {
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

impl Drop for mapblock_T {
    fn drop(&mut self) {
        self.clear_alt_or_luaref();
    }
}

#[derive(Clone)]
pub struct typebuf_T {
    pub tb_buf: Vec<u8>,
    pub tb_noremap: Vec<u8>,
    pub tb_buflen: c_int,
    pub tb_off: c_int,
    pub tb_len: c_int,
    pub tb_maplen: c_int,
    pub tb_silent: c_int,
    pub tb_no_abbr_cnt: c_int,
    pub tb_change_cnt: c_int,
}

impl typebuf_T {
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
use crate::ex_getln::{ABBR_OFF, Ctrl_RSB};
use crate::extmark::{kFalse, kNone};
use crate::insert::Ctrl_J;
use crate::state::{
    MODE_CMDLINE, MODE_INSERT, MODE_LANGMAP, MODE_NORMAL, MODE_OP_PENDING, MODE_SELECT,
    MODE_TERMINAL, MODE_VISUAL,
};

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
    first_abbr: Vec<mapblock_T>,
    maphash: [Vec<mapblock_T>; MAX_MAPHASH],
    map_luaid: c_int,
    expand_mapmodes: c_int,
    expand_isabbrev: bool,
    expand_buffer: bool,
    langmap_mapga: Vec<langmap_entry_T>,
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
const fn map_hash(mode: c_int, c1: u8) -> usize {
    if (mode & (MODE_NORMAL | MODE_VISUAL | MODE_SELECT | MODE_OP_PENDING | MODE_TERMINAL)) != 0 {
        c1 as usize
    } else {
        (c1 ^ 0x80) as usize
    }
}

use crate::keycodes::{k_second, k_third};

/// Get the start of the hashed map list for "state" and first character "c".
pub fn get_maphash_list(state: c_int, c: c_int) -> &'static [mapblock_T] {
    unsafe {
        (&raw const map_st().maphash[map_hash(state, c as u8)])
            .as_ref()
            .unwrap()
            .as_slice()
    }
}

/// Get the buffer-local hashed map list for "state" and first character "c".
pub fn get_buf_maphash_list(state: c_int, c: c_int) -> &'static [mapblock_T] {
    unsafe { (*g_st().curbuf).b_maphash[map_hash(state, c as u8)].as_slice() }
}

#[inline]
fn get_map_buckets(buf: *const buf_T, is_abbrev: bool) -> &'static [Vec<mapblock_T>] {
    unsafe {
        if is_abbrev {
            if !buf.is_null() {
                std::slice::from_ref(&(*buf).b_first_abbr)
            } else {
                std::slice::from_ref(&map_st().first_abbr)
            }
        } else if !buf.is_null() {
            &(*buf).b_maphash
        } else {
            &map_st().maphash
        }
    }
}

/// Finds the Lua mapping whose `m_str` embeds `id`.
fn map_luaid_scan(mappings: &[Vec<mapblock_T>], id: c_int) -> Option<&mapblock_T> {
    unsafe {
        for bucket in mappings {
            for mp in bucket {
                if mp.m_luaref != LUA_NOREF && crate::charset::parse_i64_prefix(std::ffi::CStr::from_ptr(mp.m_str.as_ptr().add(3)).to_bytes()) as c_int == id {
                    return Some(mp);
                }
            }
        }
        None
    }
}

/// Resolves a Lua-mapping id to its callback.
pub fn map_luaid_get(n: c_int) -> LuaRef {
    unsafe {
        let maphash_ref = &map_st().maphash;
        let first_abbr_ref = &map_st().first_abbr;
        let mut mp = map_luaid_scan(maphash_ref, n);
        if mp.is_none() {
            mp = map_luaid_scan(std::slice::from_ref(first_abbr_ref), n);
        }
        let mut fb = g_st().firstbuf;
        while !fb.is_null() {
            if mp.is_none() {
                mp = map_luaid_scan(&(*fb).b_maphash, n);
            }
            if mp.is_none() {
                mp = map_luaid_scan(std::slice::from_ref(&(*fb).b_first_abbr), n);
            }
            fb = (*fb).b_next;
        }
        mp.map_or(LUA_NOREF, |m| m.m_luaref)
    }
}

/// Put characters to represent the map mode in a string buffer.
pub fn map_mode_to_chars(mode: c_int, buf: *mut c_char) {
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

fn showmap(mp: &mapblock_T, local: bool) {
    unsafe {
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
            msg_putchar(b'\n' as c_int);
            if g_st().got_int {
                return;
            }
        }

        let mut mapchars = [0 as c_char; 7];
        map_mode_to_chars(mp.m_mode, mapchars.as_mut_ptr());
        msg_puts(mapchars.as_ptr());
        let mut len = CStr::from_ptr(mapchars.as_ptr()).to_bytes().len();

        loop {
            len += 1;
            if len > 3 {
                break;
            }
            msg_putchar(b' ' as c_int);
        }

        len = msg_outtrans_special(mp.m_keys.as_ptr(), true, 0) as usize;
        loop {
            msg_putchar(b' ' as c_int);
            len += 1;
            if len >= 12 {
                break;
            }
        }

        if mp.m_noremap == REMAP_NONE {
            msg_puts_hl(c"*".as_ptr(), HLF_8, false);
        } else if mp.m_noremap == REMAP_SCRIPT {
            msg_puts_hl(c"&".as_ptr(), HLF_8, false);
        } else {
            msg_putchar(b' ' as c_int);
        }

        if local {
            msg_putchar(b'@' as c_int);
        } else {
            msg_putchar(b' ' as c_int);
        }

        if mp.m_luaref != LUA_NOREF {
            let str_cstr = nlua_funcref_str(mp.m_luaref, true);
            msg_puts_hl(str_cstr.as_ptr(), HLF_8, false);
        } else if mp.m_str.as_bytes().is_empty() {
            msg_puts_hl(c"<Nop>".as_ptr(), HLF_8, false);
        } else {
            msg_outtrans_special(mp.m_str.as_ptr(), false, 0);
        }

        if let Some(ref desc) = mp.m_desc {
            msg_puts(c"\n                 ".as_ptr());
            msg_puts(desc.as_ptr());
        }
        if opt_vars().p_verbose > 0 {
            last_set_msg(mp.m_script_ctx);
        }
        msg_clr_eos();
    }
}

fn set_maparg_lhs_rhs(
    orig_lhs: *const c_char,
    orig_lhs_len: usize,
    orig_rhs: *const c_char,
    orig_rhs_len: usize,
    rhs_lua: LuaRef,
    cpo_val: *const c_char,
    mapargs: &mut MapArguments,
) -> bool {
    mapargs.rhs_lua = rhs_lua;

        let mut did_simplify = false;
        let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
        let Some(replaced) = replace_termcodes(
            orig_lhs,
            orig_lhs_len,
            true,
            0,
            flags,
            &mut did_simplify,
            cpo_val,
        ) else {
            return false;
        };
        mapargs.lhs_len = replaced.as_bytes().len();
        xstrlcpy(
            mapargs.lhs.as_mut_ptr(),
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
                ptr::null_mut(),
                cpo_val,
            ) else {
                return false;
            };
            mapargs.alt_lhs_len = alt_replaced.as_bytes().len();
            xstrlcpy(
                mapargs.alt_lhs.as_mut_ptr(),
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
            let orig_slice = std::slice::from_raw_parts(orig_rhs.cast::<u8>(), orig_rhs_len);
            mapargs.orig_rhs = Some(CString::new(orig_slice).unwrap());
            if CStr::from_ptr(orig_rhs)
                .to_bytes()
                .eq_ignore_ascii_case(b"<nop>")
            {
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
                    ptr::null_mut(),
                    cpo_val,
                )
                .unwrap_or_default();
                mapargs.rhs_len = replaced_cstr.as_bytes().len();
                mapargs.rhs_is_noop = orig_rhs_len != 0 && mapargs.rhs_len == 0;
                mapargs.rhs = Some(replaced_cstr);
            }
        } else {
            let mut tmp_buf = [0 as c_char; 64];
            mapargs.orig_rhs = Some(CString::default());
            mapargs.orig_rhs_len = 0;
            map_st().map_luaid += 1;
            mapargs.rhs_len = vim_snprintf!(
                tmp_buf.as_mut_ptr(),
                tmp_buf.len(),
                c"%c%c%c%d\r".as_ptr(),
                K_SPECIAL as c_int,
                KS_EXTRA as c_int,
                KE_LUA as c_int,
                map_st().map_luaid,
            ) as usize;
            mapargs.rhs = Some(CStr::from_ptr(tmp_buf.as_ptr()).to_owned());
        }
    }
}

fn str_to_mapargs(
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
        let do_backslash = vim_strchr(opt_vars().p_cpo, kCpoBslash).is_null();
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
        { let _d = (lhs_to_replace.as_mut_ptr()).cast::<u8>(); let _n = orig_lhs_len; if _n > 0 { std::ptr::copy_nonoverlapping((to_parse).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };

        let orig_rhs_len = CStr::from_ptr(rhs_start).to_bytes().len();
        if !set_maparg_lhs_rhs(
            lhs_to_replace.as_ptr(),
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
    buf: &mut buf_T,
    local: bool,
    keys: *const c_char,
    args: &MapArguments,
    noremap: c_int,
    mode: c_int,
    is_abbr: bool,
    sid: scid_T,
    lnum: linenr_T,
    simplified: bool,
    alt: Option<std::rc::Rc<MapAltLink>>,
) {
    unsafe {
        if *keys == Ctrl_C {
            if local {
                buf.b_mapped_ctrl_c |= mode;
            } else {
                g_st().mapped_ctrl_c |= mode;
            }
        }

        let m_keys = CStr::from_ptr(keys).to_owned();
        if let Some(ref alt_rc) = alt {
            alt_rc.keys[usize::from(simplified)].set(Some(m_keys.clone()));
        }
        let m_keylen = m_keys.as_bytes().len() as c_int;
        let m_script_ctx = if sid != 0 {
            sctx_T {
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
        let mp = mapblock_T {
            m_alt: alt,
            m_keys,
            m_str: args.rhs.clone().unwrap_or_default(),
            m_orig_str: args.orig_rhs.clone().unwrap_or_default(),
            m_luaref: args.rhs_lua,
            m_keylen,
            m_mode: mode,
            m_simplified: c_int::from(simplified),
            m_noremap: noremap,
            m_silent: args.silent as c_char,
            m_nowait: args.nowait as c_char,
            m_expr: args.expr as c_char,
            m_script_ctx,
            m_desc: args.desc.clone(),
            m_replace_keycodes: args.replace_keycodes,
        };

        if is_abbr {
            if local {
                buf.b_first_abbr.insert(0, mp);
            } else {
                let abbr = &raw mut map_st().first_abbr;
                (*abbr).insert(0, mp);
            }
        } else {
            let n = map_hash(mp.m_mode, *mp.m_keys.as_ptr() as u8);
            if local {
                (&mut buf.b_maphash)[n].insert(0, mp);
            } else {
                let mh = &raw mut map_st().maphash;
                (*mh)[n].insert(0, mp);
            }
        }
    }
}

fn buf_do_map(
    mut maptype: c_int,
    args: &mut MapArguments,
    mode: c_int,
    is_abbrev: bool,
    buf: &mut buf_T,
) -> c_int {
    unsafe {
        let mut retval: c_int = 0;
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

        let has_lhs = args.lhs[0] != NUL;
        let has_rhs = args.rhs_lua != LUA_NOREF
            || args.rhs.as_ref().is_some_and(|s| !s.as_bytes().is_empty())
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
                        { let _d = (keys_unescaped.as_mut_ptr()).cast::<u8>(); let _n = len as usize; if _n > 0 { std::ptr::copy_nonoverlapping((lhs).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
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
                    g_st().no_abbr = false;
                }

                if do_print {
                    msg_start();
                }

                if args.unique && local && has_lhs && has_rhs && maptype != MAPTYPE_UNMAP {
                    let lhs_slice = std::slice::from_raw_parts(lhs.cast::<u8>(), len as usize);
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
                                    let lhs_slice =
                                        std::slice::from_raw_parts(lhs.cast::<u8>(), cmp_len);
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
                            map_hash(mode, *lhs as u8)
                        };
                        (start, start + 1)
                    } else {
                        (0, 256)
                    };
                    for hash in hash_start..hash_end {
                        if g_st().got_int {
                            break;
                        }
                        let mpp: &mut Vec<mapblock_T> = if is_abbrev {
                            if local {
                                &mut buf.b_first_abbr
                            } else {
                                let abbr = &raw mut map_st().first_abbr;
                                &mut *abbr
                            }
                        } else if local {
                            let bmh = &raw mut buf.b_maphash;
                            &mut (*bmh)[hash]
                        } else {
                            let mh = &raw mut map_st().maphash;
                            &mut (*mh)[hash]
                        };
                        let mut idx: usize = 0;
                        while idx < mpp.len() && !g_st().got_int {
                            let mp: &mut mapblock_T = &mut mpp[idx];
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
                                    (mp.m_str.as_bytes().len() as c_int, mp.m_str.as_ptr())
                                } else {
                                    (mp.m_keylen, mp.m_keys.as_ptr())
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
                                            mp.m_nowait = args.nowait as c_char;
                                            mp.m_silent = args.silent as c_char;
                                            mp.m_mode = mode;
                                            mp.m_simplified = c_int::from(keyround1_simplified);
                                            mp.m_expr = args.expr as c_char;
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

                                    let new_hash = map_hash(mp.m_mode, *mp.m_keys.as_ptr() as u8);
                                    if !is_abbrev && new_hash != hash {
                                        let moved = mpp.remove(idx);
                                        if local {
                                            let bmh = &raw mut buf.b_maphash;
                                            (*bmh)[new_hash].insert(0, moved);
                                        } else {
                                            let mh = &raw mut map_st().maphash;
                                            (*mh)[new_hash].insert(0, moved);
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
                    } else if *lhs == Ctrl_C {
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

                map_add(
                    buf,
                    local,
                    lhs,
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
pub fn do_map(maptype: c_int, arg: *mut c_char, mode: c_int, is_abbrev: bool) -> c_int {
    unsafe {
        let mut parsed_args = MapArguments::new();
        let mut result = str_to_mapargs(arg, maptype == MAPTYPE_UNMAP, &mut parsed_args);
        match result {
            0 => {
                result = buf_do_map(maptype, &mut parsed_args, mode, is_abbrev, &mut *g_st().curbuf);
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
fn get_map_mode(cmdp: *mut *mut c_char, forceit: bool) -> c_int {
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

fn do_mapclear(mut cmdp: *mut c_char, arg: *mut c_char, forceit: c_int, abbr: c_int) {
    unsafe {
        let local = CStr::from_ptr(arg).to_bytes() == b"<buffer>";
        if !local && *arg != NUL {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mode = get_map_mode(&mut cmdp, forceit != 0);
        map_clear_mode(g_st().curbuf, mode, local, abbr != 0);
    }
}

/// Clear all mappings in "mode".
pub fn map_clear_mode(buf: *mut buf_T, mode: c_int, local: bool, abbr: bool) {
    unsafe {
        for hash in 0..256 {
            let mpp: &mut Vec<mapblock_T> = if abbr {
                if hash > 0 {
                    break;
                }
                if local {
                    &mut (*buf).b_first_abbr
                } else {
                    let a = &raw mut map_st().first_abbr;
                    &mut *a
                }
            } else if local {
                &mut (&mut (*buf).b_maphash)[hash]
            } else {
                let mh = &raw mut map_st().maphash;
                &mut (*mh)[hash]
            };
            let mut idx: usize = 0;
            while idx < mpp.len() {
                let mp: &mut mapblock_T = &mut mpp[idx];
                if (mp.m_mode & mode) != 0 {
                    mp.m_mode &= !mode;
                    if mp.m_mode == 0 {
                        mpp.remove(idx);
                        continue;
                    }
                    let new_hash = map_hash(mp.m_mode, *mp.m_keys.as_ptr() as u8);
                    if !abbr && new_hash != hash {
                        let moved = mpp.remove(idx);
                        if local {
                            (&mut (*buf).b_maphash)[new_hash].insert(0, moved);
                        } else {
                            let mh = &raw mut map_st().maphash;
                            (*mh)[new_hash].insert(0, moved);
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
pub fn map_to_exists(str_ptr: *const c_char, modechars: *const c_char, abbr: bool) -> bool {
    unsafe {
        let mut mode: c_int = 0;
        let rhs = replace_termcodes(
            str_ptr,
            CStr::from_ptr(str_ptr).to_bytes().len(),
            false,
            0,
            REPTERM_DO_LT,
            ptr::null_mut(),
            opt_vars().p_cpo,
        )
        .unwrap_or_default();

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

        map_to_exists_mode(rhs.as_ptr(), mode, abbr)
    }
}

/// Check if a map exists that has given string in the rhs.
pub fn map_to_exists_mode(rhs: *const c_char, mode: c_int, abbr: bool) -> bool {
    unsafe {
        for exp_buffer in [false, true] {
            let b = if exp_buffer { g_st().curbuf } else { ptr::null() };
            for bucket in get_map_buckets(b, abbr) {
                for mp in bucket {
                    let rhs_b = CStr::from_ptr(rhs).to_bytes();
            if (mp.m_mode & mode) != 0 && (rhs_b.is_empty() || mp.m_str.as_bytes().windows(rhs_b.len()).any(|w| w == rhs_b)) {
                        return true;
                    }
                }
            }
        }

        false
    }
}

fn translate_mapping(str_in: *const c_char, cpo_val: *const c_char) -> CString {
    unsafe {
        let mut str_ptr = str_in.cast::<u8>();
        let mut buf: Vec<u8> = Vec::with_capacity(40);

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
                    c = to_special(*str_ptr.add(1) as c_int, *str_ptr.add(2) as c_int);
                    if c == K_ZERO {
                        c = NUL as c_int;
                    }
                    str_ptr = str_ptr.add(2);
                }
                if is_special(c) || modifiers != 0 {
                    let s = get_special_key_name(c, modifiers);
                    if !s.is_null() {
                        buf.extend_from_slice(CStr::from_ptr(s).to_bytes());
                    }
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
                buf.push(if cpo_bslash { Ctrl_V as u8 } else { b'\\' });
            }

            if c != 0 {
                buf.push(c as u8);
            }
            str_ptr = str_ptr.add(1);
        }
        crate::memory::cstr_from_bytes(&buf)
    }
}

/// Work out what to complete when doing command line completion of mapping
/// or abbreviation names.
pub fn set_context_in_map_cmd(
    xp: &mut expand_T,
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
            xp.xp_context = EXPAND_NOTHING;
        } else {
            if isunmap {
                map_st().expand_mapmodes = get_map_mode(&mut cmd, forceit || isabbrev);
            } else {
                map_st().expand_mapmodes = MODE_INSERT | MODE_CMDLINE;
                if !isabbrev {
                    map_st().expand_mapmodes |= MODE_VISUAL | MODE_SELECT | MODE_NORMAL | MODE_OP_PENDING;
                }
            }
            map_st().expand_isabbrev = isabbrev;
            xp.xp_context = EXPAND_MAPPINGS;
            map_st().expand_buffer = false;
            loop {
                let bytes = CStr::from_ptr(arg).to_bytes();
                if bytes.starts_with(b"<buffer>") {
                    map_st().expand_buffer = true;
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
            xp.xp_pattern = arg;
        }

        ptr::null_mut()
    }
}

/// Find all mapping/abbreviation names that match regexp "regmatch".
pub fn ExpandMappings(
    pat: *mut c_char,
    regmatch: &mut regmatch_T,
    matches: &mut Vec<CString>,
) -> c_int {
    let fuzzy = cmdline_fuzzy_complete(pat);

        matches.clear();

        let mut str_matches: Vec<CString> = Vec::new();
        let mut fuz_matches: Vec<fuzmatch_str_T> = Vec::new();

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
                let idx = fuz_matches.len() as c_int;
                fuz_matches.push(fuzmatch_str_T {
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

                let mut score: c_int = 0;
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
                    let idx = fuz_matches.len() as c_int;
                    fuz_matches.push(fuzmatch_str_T {
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
            return FAIL;
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

        if matches.is_empty() { FAIL } else { OK }
}

/// Check for an abbreviation.
pub fn check_abbr(
    mut c: c_int,
    mut ptr_arg: *mut c_char,
    col: c_int,
    mincol: c_int,
) -> bool {
    unsafe {
        let mut tb = [0u8; MB_MAXBYTES + 4];

        if g_st().typebuf.tb_no_abbr_cnt != 0 {
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
            let mut mp_opt: Option<&mapblock_T> = None;
            let first_abbr_slice = map_st().first_abbr.as_slice();
            for cand in (*g_st().curbuf).b_first_abbr.iter().chain(first_abbr_slice) {
                let mut qlen = cand.m_keylen;
                let mut q_vec: Vec<c_char>;
                let mut q = cand.m_keys.as_ptr().cast_mut();

                if cand.m_keys.as_bytes().contains(&K_SPECIAL) {
                    q_vec = cand
                        .m_keys
                        .as_bytes_with_nul()
                        .iter()
                        .map(|&b| b as c_char)
                        .collect();
                    q = q_vec.as_mut_ptr();
                    qlen = vim_unescape_ks(q) as c_int;
                }
                let match_found = (cand.m_mode & g_st().State) != 0
                    && qlen == len
                    && std::slice::from_raw_parts(q.cast::<u8>(), len as usize)
                        == std::slice::from_raw_parts(ptr_arg.cast::<u8>(), len as usize);
                if match_found {
                    mp_opt = Some(cand);
                    break;
                }
            }
            if let Some(mp) = mp_opt {
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
                        let escaped_bytes = escaped.to_bytes();
                        newlen = escaped_bytes.len();
                        ptr::copy(escaped_bytes.as_ptr(), tb.as_mut_ptr().add(j), newlen);
                        j += newlen;
                    }
                    tb[j] = 0;
                    ins_typebuf(
                        tb.as_mut_ptr().cast::<c_char>(),
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

                let owned_s = if expr { eval_map_expr(mp, c) } else { m_str_clone };
                if let Some(ref cs) = owned_s {
                    let s = cs.as_ptr().cast_mut();
                    ins_typebuf(s, noremap, 0, true, silent);
                    g_st().typebuf.tb_no_abbr_cnt += (cs.to_bytes().len() + j + 1) as c_int;
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
pub fn eval_map_expr(mp: &mapblock_T, c: c_int) -> Option<CString> {
    unsafe {
        let mut p: Option<CString> = None;
        let mut expr_bytes: Vec<c_char> = Vec::new();
        let luaref = mp.m_luaref;
        let replace_keycodes = mp.m_replace_keycodes;

        if luaref == LUA_NOREF {
            expr_bytes.extend_from_slice(std::slice::from_raw_parts(
                mp.m_str.as_ptr(),
                mp.m_str.to_bytes_with_nul().len(),
            ));
            vim_unescape_ks(expr_bytes.as_mut_ptr());
        }

        g_st().expr_map_lock += 1;
        set_vim_var_char(c);
        let save_cursor = (**curwin.as_ptr()).w_cursor;
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
                kRetObject,
                ptr::null_mut(),
                &mut err,
            );
            if ret.obj_type == ObjectType::kObjectTypeString {
                p = Some(string_to_cstr(ret.data.string()));
            }
            api_free_object(ret);
            if err.err_type != ErrorType::kErrorTypeNone {
                semsg_multiline!(c"emsg".as_ptr(), c"E5108: %s".as_ptr(), err.msg_ptr());
                api_clear_error(&mut err);
            }
        } else {
            p = eval_to_string(expr_bytes.as_mut_ptr(), false, false);
        }
        g_st().expr_map_lock -= 1;
        (**curwin.as_ptr()).w_cursor = save_cursor;
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
                ptr::null_mut(),
                opt_vars().p_cpo,
            )
        } else {
            Some(vim_strsave_escape_ks(p.as_ptr()))
        }
    }
}

/// Write map commands for the current mappings to an .exrc file.
pub fn makemap(fd: *mut FILE, buf: *mut buf_T) -> c_int {
    unsafe {
        let mut did_cpo = false;

        for abbr in 0..2 {
            for bucket in get_map_buckets(buf, abbr != 0) {
                for mp in bucket {
                    if mp.m_simplified != 0
                        || mp.m_noremap == REMAP_SCRIPT
                        || mp.m_luaref != LUA_NOREF
                    {
                        continue;
                    }

                    let mut p = mp.m_str.as_ptr();
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

                    match mp.m_mode {
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
                        if mp.m_noremap != REMAP_YES && fputs(c"nore".as_ptr(), fd) < 0 {
                            return FAIL;
                        }
                        if fputs(cmd, fd) < 0 {
                            return FAIL;
                        }
                        if !buf.is_null() && fputs(c" <buffer>".as_ptr(), fd) < 0 {
                            return FAIL;
                        }
                        if mp.m_nowait != 0 && fputs(c" <nowait>".as_ptr(), fd) < 0 {
                            return FAIL;
                        }
                        if mp.m_silent != 0 && fputs(c" <silent>".as_ptr(), fd) < 0 {
                            return FAIL;
                        }
                        if mp.m_expr != 0 && fputs(c" <expr>".as_ptr(), fd) < 0 {
                            return FAIL;
                        }

                        if fputc(b' ' as c_int, fd) < 0
                            || put_escstr(fd, mp.m_keys.as_ptr(), 0) == FAIL
                            || fputc(b' ' as c_int, fd) < 0
                            || put_escstr(fd, mp.m_str.as_ptr(), 1) == FAIL
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
pub fn put_escstr(fd: *mut FILE, strstart: *const c_char, what: c_int) -> c_int {
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
                    c = to_special(*str_ptr.add(1) as c_int, *str_ptr.add(2) as c_int);
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
pub fn check_map(
    keys: *mut c_char,
    mode: c_int,
    exact: c_int,
    ign_mod: c_int,
    abbr: c_int,
    mut mp_out: Option<&mut Option<&'static mapblock_T>>,
    local_ptr: *mut c_int,
    rhs_lua: *mut c_int,
) -> *mut c_char {
    unsafe {
        *rhs_lua = LUA_NOREF;

        let len = CStr::from_ptr(keys).to_bytes().len() as c_int;
        for local in (0..=1).rev() {
            let b = if local != 0 { g_st().curbuf } else { ptr::null() };
            for bucket in get_map_buckets(b, abbr != 0) {
                for mp in bucket {
                    if (mp.m_mode & mode) != 0 && (exact == 0 || mp.m_keylen == len) {
                        let mut s = mp.m_keys.as_ptr();
                        let mut keylen = mp.m_keylen;
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
                            if let Some(ref mut out) = mp_out {
                                **out = Some(mp);
                            }
                            if !local_ptr.is_null() {
                                *local_ptr = local;
                            }
                            *rhs_lua = mp.m_luaref;
                            return if mp.m_luaref == LUA_NOREF {
                                mp.m_str.as_ptr().cast_mut()
                            } else {
                                ptr::null_mut()
                            };
                        }
                    }
                }
            }
        }

        ptr::null_mut()
    }
}

/// "hasmapto()" function
pub fn f_hasmapto(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    let argvars = argvars.as_mut_ptr();

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

        rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(map_to_exists(name, mode, abbr)));
    }
}

fn mapblock_fill_dict(
    mp: &mapblock_T,
    lhsrawalt: *const c_char,
    buffer_value: c_int,
    abbr: bool,
    compatible: bool,
    arena: *mut Arena,
) -> Dict {
    let mut dict = arena_dict(arena, 20);
        let lhs = str2special_arena(
            mp.m_keys.as_ptr(),
            compatible,
            if compatible { kFalse } else { kNone },
            arena,
        );
        let mapmode = arena_alloc(arena, 7, false).cast::<c_char>();
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
                mp.m_orig_str.as_ptr()
            } else {
                str2special_arena(mp.m_str.as_ptr(), false, kNone, arena)
            });
            put_c(&mut dict, c"rhs", string_obj(rhs));
        }
        if let Some(ref desc) = mp.m_desc {
            put_c(&mut dict, c"desc", cstr_as_obj(desc.as_ptr()));
        }
        put_c(&mut dict, c"lhs", cstr_as_obj(lhs));
        put_c(&mut dict, c"lhsraw", cstr_as_obj(mp.m_keys.as_ptr()));
        if !lhsrawalt.is_null() {
            put_c(&mut dict, c"lhsrawalt", cstr_as_obj(lhsrawalt));
        }
        put_c(&mut dict, c"noremap", integer_obj(noremap_value));
        put_c(
            &mut dict,
            c"script",
            integer_obj(i64::from(mp.m_noremap == REMAP_SCRIPT)),
        );
        put_c(
            &mut dict,
            c"expr",
            integer_obj(i64::from(mp.m_expr != 0)),
        );
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
        put_c(&mut dict, c"mode", cstr_as_obj(mapmode));
        put_c(&mut dict, c"abbr", integer_obj(i64::from(abbr)));
        put_c(
            &mut dict,
            c"mode_bits",
            integer_obj(i64::from(mp.m_mode)),
        );

        dict
}

fn get_maparg(argvars: *mut typval_T, rettv: &mut typval_T, exact: c_int) {
    unsafe {
        rettv.v_type = VarType::VAR_STRING;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

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

        let mut did_simplify = false;
        let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
        let mut which_mut = which as *mut c_char;
        let mode = get_map_mode(&mut which_mut, false);

        let keys_len = CStr::from_ptr(keys).to_bytes().len();
        let keys_simplified = replace_termcodes(
            keys,
            keys_len,
            false,
            0,
            flags,
            &mut did_simplify,
            opt_vars().p_cpo,
        )
        .unwrap_or_default();
        let mut mp_opt: Option<&'static mapblock_T> = None;
        let mut buffer_local: c_int = 0;
        let mut rhs_lua: LuaRef = LUA_NOREF;
        let mut rhs = check_map(
            keys_simplified.as_ptr().cast_mut(),
            mode,
            exact,
            0,
            c_int::from(abbr),
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
                ptr::null_mut(),
                opt_vars().p_cpo,
            )
            .unwrap_or_default();
            rhs = check_map(
                alt_keys.as_ptr().cast_mut(),
                mode,
                exact,
                0,
                c_int::from(abbr),
                Some(&mut mp_opt),
                &mut buffer_local,
                &mut rhs_lua,
            );
        }

        if !get_dict {
            if !rhs.is_null() {
                if *rhs == NUL {
                    crate::eval::typval::tv_set_string(&mut *rettv, c"<Nop>".as_ptr());
                } else {
                    crate::eval::typval::tv_set_string(
                        &mut *rettv,
                        str2special_save(rhs, false, kFalse).as_ptr(),
                    );
                }
            } else if rhs_lua != LUA_NOREF
                && let Some(mp) = mp_opt
            {
                let s = nlua_funcref_str(mp.m_luaref, true);
                crate::eval::typval::tv_set_string_len(&mut *rettv, s.as_ptr(), s.as_bytes().len());
            }
        } else if let Some(mp) = mp_opt
            && (!rhs.is_null() || rhs_lua != LUA_NOREF)
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
            object_to_vim_take_luaref(&mut dobj, &raw mut *rettv, true, &mut crate::api::private::validate::Error::new());
            arena_mem_free(arena_finish(&mut arena));
        } else {
            tv_dict_alloc_ret(&raw mut *rettv);
        }
    }
}

fn get_map_mode_string(mode_string: *const c_char, abbr: bool) -> c_int {
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
pub fn f_mapset(argvars: &mut [typval_T], _rettv: &mut typval_T, _fptr: EvalFuncData){
    let argvars = argvars.as_mut_ptr();

    unsafe {
        if check_secure() {
            return;
        }

        let mut buf = [0 as c_char; NUMBUFLEN];
        let dict_only = (*argvars).v_type == VarType::VAR_DICT;

        let (which, is_abbr, d) = if dict_only {
            let d = (*argvars).vval.v_dict();
            let which = tv_dict_get_string(d, c"mode".as_ptr());
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
            let d = (*argvars.add(2)).vval.v_dict();
            (which, is_abbr, d)
        };
        let mode = get_map_mode_string(which, is_abbr != 0);
        if mode == 0 {
            semsg!(gettext(E_ILLEGAL_MAP_MODE_STRING_STR.as_ptr()), which);
            return;
        }

        let lhs = tv_dict_get_string(d, c"lhs".as_ptr());
        let lhsraw = tv_dict_get_string(d, c"lhsraw".as_ptr());
        let lhsrawalt = tv_dict_get_string(d, c"lhsrawalt".as_ptr());
        let mut orig_rhs = tv_dict_get_string(d, c"rhs".as_ptr()) as *const c_char;
        let mut rhs_lua: LuaRef = LUA_NOREF;
        let callback_di = tv_dict_find(d, c"callback".as_ptr(), 8);
        if !callback_di.is_null() && (*callback_di).di_tv.v_type == VarType::VAR_FUNC {
            let fp = find_func((*callback_di).di_tv.vval.v_string());
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
        args.desc = crate::eval::typval::tv_dict_get_cstring(d, c"desc".as_ptr());

        let sid = tv_dict_get_number(d, c"sid".as_ptr()) as scid_T;
        let lnum = tv_dict_get_number(d, c"lnum".as_ptr()) as linenr_T;
        let buffer = tv_dict_get_number(d, c"buffer".as_ptr()) != 0;

        set_maparg_rhs(
            orig_rhs,
            CStr::from_ptr(orig_rhs).to_bytes().len(),
            rhs_lua,
            sid,
            opt_vars().p_cpo,
            &mut args,
        );

        let mut unmap_args = MapArguments::new();
        set_maparg_lhs_rhs(
            lhs,
            CStr::from_ptr(lhs).to_bytes().len(),
            c"".as_ptr(),
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
            &mut *g_st().curbuf,
        );

        let alt = (!lhsrawalt.is_null()).then(|| {
            std::rc::Rc::new(MapAltLink {
                keys: [std::cell::Cell::new(None), std::cell::Cell::new(None)],
            })
        });
        map_add(
            &mut *g_st().curbuf,
            buffer,
            lhsraw,
            &args,
            noremap,
            mode,
            is_abbr != 0,
            sid,
            lnum,
            false,
            alt.clone(),
        );
        if !lhsrawalt.is_null() {
            map_add(
                &mut *g_st().curbuf,
                buffer,
                lhsrawalt,
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
pub fn f_maplist(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    let argvars = argvars.as_mut_ptr();

    unsafe {
        let flags = REPTERM_FROM_PART | REPTERM_DO_LT;
        let abbr = (*argvars).v_type != VarType::VAR_UNKNOWN && tv_get_bool(argvars) != 0;

        tv_list_alloc_ret(rettv, kListLenUnknown);

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
                    let lhs = str2special_arena(mp.m_keys.as_ptr(), true, kFalse, &mut arena);
                    let keys_buf = replace_termcodes(
                        lhs,
                        CStr::from_ptr(lhs).to_bytes().len(),
                        false,
                        0,
                        flags,
                        &mut did_simplify,
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
                    let mut d = typval_T {
                        v_type: VarType::VAR_UNKNOWN,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::Number(0),
                    };
                    let mut dobj = dict_obj(dict);
                    object_to_vim_take_luaref(&mut dobj, &mut d, true, &mut crate::api::private::validate::Error::new());
                    debug_assert!(d.v_type == VarType::VAR_DICT);
                    tv_list_append_dict(rettv.vval.v_list(), d.vval.v_dict());
                    arena_mem_free(arena_finish(&mut arena));
                }
            }
        }
    }
}

/// "maparg()" function
pub fn f_maparg(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    let argvars = argvars.as_mut_ptr();

    get_maparg(argvars, rettv, 1);
}

/// "mapcheck()" function
pub fn f_mapcheck(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    let argvars = argvars.as_mut_ptr();

    get_maparg(argvars, rettv, 0);
}

/// Add a mapping.
pub fn add_map(lhs: *mut c_char, rhs: *mut c_char, mode: c_int, buffer: bool) {
    unsafe {
        let mut args = MapArguments::new();
        set_maparg_lhs_rhs(
            lhs,
            CStr::from_ptr(lhs).to_bytes().len(),
            rhs,
            CStr::from_ptr(rhs).to_bytes().len(),
            LUA_NOREF,
            opt_vars().p_cpo,
            &mut args,
        );
        args.buffer = buffer;

        buf_do_map(MAPTYPE_NOREMAP, &mut args, mode, false, &mut *g_st().curbuf);
    }
}

fn langmap_set_entry(from: c_int, to: c_int) {
    unsafe {
        let ptr = &raw mut map_st().langmap_mapga;
        let entries = &mut *ptr;
        match entries.binary_search_by_key(&from, |e| e.from) {
            Ok(idx) => entries[idx].to = to,
            Err(idx) => entries.insert(idx, langmap_entry_T { from, to }),
        }
    }
}

/// Apply 'langmap' to multi-byte character "c" and return the result.
pub fn langmap_adjust_mb(c: c_int) -> c_int {
    let entries = map_st().langmap_mapga.as_slice();
    entries
        .binary_search_by_key(&c, |e| e.from)
        .map_or(c, |idx| entries[idx].to)
}

pub fn langmap_init() {
    unsafe {
        for i in 0..256 {
            *(&raw mut g_st().langmap_mapchar).cast::<u8>().add(i) = i as u8;
        }
        map_st().langmap_mapga = Vec::new();
    }
}

/// Called when langmap option is set; the language map can be
/// changed at any time!
pub fn did_set_langmap(args: &mut optset_T) -> *const c_char {
    unsafe {
        langmap_init();

        let mut p = opt_vars().p_langmap;
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
                        args.os_errbuf,
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
                    g_st().langmap_mapchar[(from & 255) as usize] = to as u8;
                }

                p = p.add(utfc_ptr2len(p) as usize);
                if !p2.is_null() {
                    p2 = p2.add(utfc_ptr2len(p2) as usize);
                    if *p == b';' as c_char {
                        p = p2;
                        if *p != NUL {
                            if *p != b',' as c_char {
                                return opt_error(
                                    args.os_errbuf,
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

fn do_exmap(eap: &mut exarg_T, isabbrev: bool) {
    unsafe {
        let mut cmdp = eap.cmd;
        let mode = get_map_mode(&mut cmdp, eap.forceit != 0 || isabbrev);

        let maptype = if *cmdp == b'n' as c_char {
            MAPTYPE_NOREMAP
        } else if *cmdp == b'u' as c_char {
            MAPTYPE_UNMAP
        } else {
            MAPTYPE_MAP
        };
        let mut parsed_args = MapArguments::new();
        let result = str_to_mapargs(eap.arg, maptype == MAPTYPE_UNMAP, &mut parsed_args);
        match result {
            0 => match buf_do_map(maptype, &mut parsed_args, mode, isabbrev, &mut *g_st().curbuf) {
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
    }
}

/// ":abbreviate" and friends.
pub fn ex_abbreviate(eap: &mut exarg_T) {
    do_exmap(eap, true);
}

/// ":map" and friends.
pub fn ex_map(eap: &mut exarg_T) {
    if g_st().secure != 0 {
            g_st().secure = 2;
            msg_outtrans(eap.cmd, 0, false);
            msg_putchar(b'\n' as c_int);
        }
        do_exmap(eap, false);
}

/// ":unmap" and friends.
pub fn ex_unmap(eap: &mut exarg_T) {
    do_exmap(eap, false);
}

/// ":mapclear" and friends.
pub fn ex_mapclear(eap: &mut exarg_T) {
    do_mapclear(eap.cmd, eap.arg, eap.forceit, 0);
}

/// ":abclear" and friends.
pub fn ex_abclear(eap: &mut exarg_T) {
    do_mapclear(eap.cmd, eap.arg, 1, 1);
}

/// Set, tweak, or remove a mapping in a mode.
pub fn modify_keymap(
    channel_id: u64,
    mut buffer: Buffer,
    mut maptype: c_int,
    mode: NvimString,
    lhs: NvimString,
    rhs: NvimString,
    opts: *mut KeyDict_keymap,
    err: &mut Error,
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
                parsed_args.desc = Some(string_to_cstr((*opts).desc));
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
                opt_vars().p_cpo,
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
                        parsed_args
                            .rhs
                            .as_ref()
                            .map_or(c"".as_ptr(), |s| s.as_ptr()),
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

            match buf_do_map(maptype, &mut parsed_args, mode_val, is_abbrev, &mut *target_buf) {
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

        g_st().current_sctx = save_current_sctx;
        nlua_clear_ref(&mut parsed_args.rhs_lua);
    }
}

/// Get an array containing dictionaries describing mappings
/// based on mode and buffer id.
pub fn keymap_array(mode: NvimString, buf: *mut buf_T, arena: *mut Arena) -> Array {
    unsafe {
        let mut mappings = ArrayBuilder::new();

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_mode_to_chars_and_langmap_init_work() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            let mut buf = [0 as c_char; 16];
            map_mode_to_chars(MODE_NORMAL | MODE_VISUAL, buf.as_mut_ptr());
            assert_eq!(CStr::from_ptr(buf.as_ptr()).to_bytes(), b"nx");

            langmap_init();
            assert_eq!(langmap_adjust_mb(b'a' as c_int), b'a' as c_int);
        }
    }
}
