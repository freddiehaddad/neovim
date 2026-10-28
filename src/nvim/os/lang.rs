//! Locale and message language management (`src/nvim/os/lang.c`).

use crate::buffer::maketitle;
use crate::charset::{
    buf_init_isk_chartab, init_isf_chartab, init_isi_chartab, skiptowhite, skipwhite,
};
use crate::eval::vars::{
    VV_COLLATE, VV_CTYPE, VV_LANG, VV_LC_TIME, VV_PROGPATH, get_vim_var_str, set_vim_var_string,
};
use crate::ex_docmd::Exarg;
use crate::globals::*;
use crate::mbyte::{Vimconv, convert_setup, mb_strnicmp, string_convert};
use crate::memory::xstrlcpy;
use crate::message::{semsg, smsg};
use crate::option::set_helplang_default;
use crate::os::env::{ExpandCtx, os_getenv_buf, os_setenv};
#[cfg(unix)]
use crate::os::shell::get_cmd_output;
use crate::path::{path_tail, path_tail_with_sep};
use crate::profile::time_msg;
use crate::strings::gettext;
use std::ffi::{CStr, CString, c_char, c_int};
use std::path::{Path, PathBuf};
use std::ptr;

pub(crate) const HAVE_LIBINTL: bool = match option_env!("NVIM_HAVE_LIBINTL") {
    Some(s) => !matches!(s.as_bytes(), b"0" | b"false" | b"FALSE" | b"off" | b"OFF"),
    None => true,
};

pub(crate) const LC_CTYPE: c_int = if cfg!(target_os = "linux") { 0 } else { 2 };

#[cfg(windows)]
const LC_ALL: c_int = 0;
#[cfg(windows)]
const LC_COLLATE: c_int = 1;
#[cfg(windows)]
const LC_NUMERIC: c_int = 4;
#[cfg(windows)]
const LC_TIME: c_int = 5;
#[cfg(windows)]
const VIM_LC_MESSAGES: c_int = 6789;

#[cfg(target_os = "linux")]
const LC_NUMERIC: c_int = 1;
#[cfg(target_os = "linux")]
const LC_TIME: c_int = 2;
#[cfg(target_os = "linux")]
const LC_COLLATE: c_int = 3;
#[cfg(target_os = "linux")]
const LC_MESSAGES: c_int = 5;
#[cfg(target_os = "linux")]
const LC_ALL: c_int = 6;
#[cfg(target_os = "linux")]
const VIM_LC_MESSAGES: c_int = LC_MESSAGES;

#[cfg(not(any(windows, target_os = "linux")))]
const LC_ALL: c_int = 0;
#[cfg(not(any(windows, target_os = "linux")))]
const LC_COLLATE: c_int = 1;
#[cfg(not(any(windows, target_os = "linux")))]
const LC_NUMERIC: c_int = 4;
#[cfg(not(any(windows, target_os = "linux")))]
const LC_TIME: c_int = 5;
#[cfg(not(any(windows, target_os = "linux")))]
const LC_MESSAGES: c_int = 6;
#[cfg(not(any(windows, target_os = "linux")))]
const VIM_LC_MESSAGES: c_int = LC_MESSAGES;

unsafe extern "C" {
    pub(crate) fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;

    #[cfg(unix)]
    pub(crate) fn nl_langinfo(item: c_int) -> *mut c_char;
}

struct MoEntry {
    msgid: Box<[u8]>,
    forms: Box<[CString]>,
}

struct MoCatalog {
    path: PathBuf,
    plural_expr: Box<[u8]>,
    entries: Box<[MoEntry]>,
}

struct LangModuleState {
    locales: Vec<CString>,
    #[cfg(not(windows))]
    did_init_locales: bool,
    locale_dir: [u8; MAXPATHL],
    locale_dir_len: usize,
    catalogs: Vec<MoCatalog>,
    active_catalog: Option<usize>,
    catalog_valid: bool,
}

impl LangModuleState {
    const fn new() -> Self {
        Self {
            locales: Vec::new(),
            #[cfg(not(windows))]
            did_init_locales: false,
            locale_dir: [0; MAXPATHL],
            locale_dir_len: 0,
            catalogs: Vec::new(),
            active_catalog: None,
            catalog_valid: false,
        }
    }
}

static LANG_STATE: crate::globals::GlobalState<LangModuleState> =
    crate::globals::GlobalState::new(LangModuleState::new());

#[inline]
fn lang_st() -> &'static mut LangModuleState {
    unsafe { &mut *LANG_STATE.as_ptr() }
}

fn eval_plural_expr(expr: &[u8], n: u64) -> usize {
    struct Parser<'a> {
        s: &'a [u8],
        pos: usize,
        n: u64,
    }
    impl Parser<'_> {
        fn skip_ws(&mut self) {
            while self.pos < self.s.len()
                && matches!(self.s[self.pos], b' ' | b'\t' | b'\r' | b'\n')
            {
                self.pos += 1;
            }
        }
        fn eat(&mut self, pat: &[u8]) -> bool {
            self.skip_ws();
            if self.s[self.pos..].starts_with(pat) {
                self.pos += pat.len();
                true
            } else {
                false
            }
        }
        fn parse_ternary(&mut self) -> u64 {
            let cond = self.parse_or();
            if self.eat(b"?") {
                let then_v = self.parse_ternary();
                let _ = self.eat(b":");
                let else_v = self.parse_ternary();
                if cond != 0 { then_v } else { else_v }
            } else {
                cond
            }
        }
        fn parse_or(&mut self) -> u64 {
            let mut v = self.parse_and();
            while self.eat(b"||") {
                let rhs = self.parse_and();
                v = u64::from(v != 0 || rhs != 0);
            }
            v
        }
        fn parse_and(&mut self) -> u64 {
            let mut v = self.parse_eq();
            while self.eat(b"&&") {
                let rhs = self.parse_eq();
                v = u64::from(v != 0 && rhs != 0);
            }
            v
        }
        fn parse_eq(&mut self) -> u64 {
            let mut v = self.parse_rel();
            loop {
                if self.eat(b"==") {
                    let rhs = self.parse_rel();
                    v = u64::from(v == rhs);
                } else if self.eat(b"!=") {
                    let rhs = self.parse_rel();
                    v = u64::from(v != rhs);
                } else {
                    break;
                }
            }
            v
        }
        fn parse_rel(&mut self) -> u64 {
            let mut v = self.parse_add();
            loop {
                if self.eat(b"<=") {
                    let rhs = self.parse_add();
                    v = u64::from(v <= rhs);
                } else if self.eat(b">=") {
                    let rhs = self.parse_add();
                    v = u64::from(v >= rhs);
                } else if self.eat(b"<") {
                    let rhs = self.parse_add();
                    v = u64::from(v < rhs);
                } else if self.eat(b">") {
                    let rhs = self.parse_add();
                    v = u64::from(v > rhs);
                } else {
                    break;
                }
            }
            v
        }
        fn parse_add(&mut self) -> u64 {
            let mut v = self.parse_mul();
            loop {
                if self.eat(b"+") {
                    v = v.wrapping_add(self.parse_mul());
                } else if self.eat(b"-") {
                    v = v.wrapping_sub(self.parse_mul());
                } else {
                    break;
                }
            }
            v
        }
        fn parse_mul(&mut self) -> u64 {
            let mut v = self.parse_unary();
            loop {
                if self.eat(b"%") {
                    let rhs = self.parse_unary();
                    v = v.checked_rem(rhs).unwrap_or(0);
                } else if self.eat(b"*") {
                    v = v.wrapping_mul(self.parse_unary());
                } else if self.eat(b"/") {
                    let rhs = self.parse_unary();
                    v = v.checked_div(rhs).unwrap_or(0);
                } else {
                    break;
                }
            }
            v
        }
        fn parse_unary(&mut self) -> u64 {
            self.skip_ws();
            if self.pos < self.s.len()
                && self.s[self.pos] == b'!'
                && self.s.get(self.pos + 1) != Some(&b'=')
            {
                self.pos += 1;
                u64::from(self.parse_unary() == 0)
            } else {
                self.parse_primary()
            }
        }
        fn parse_primary(&mut self) -> u64 {
            self.skip_ws();
            if self.eat(b"(") {
                let v = self.parse_ternary();
                let _ = self.eat(b")");
                return v;
            }
            if self.pos < self.s.len() && self.s[self.pos] == b'n' {
                self.pos += 1;
                return self.n;
            }
            let mut val: u64 = 0;
            while self.pos < self.s.len() && self.s[self.pos].is_ascii_digit() {
                val = val
                    .wrapping_mul(10)
                    .wrapping_add(u64::from(self.s[self.pos] - b'0'));
                self.pos += 1;
            }
            val
        }
    }
    if expr.is_empty() {
        return usize::from(n != 1);
    }
    let mut p = Parser { s: expr, pos: 0, n };
    p.parse_ternary() as usize
}

fn parse_mo_header(header: &[u8]) -> (Option<Vec<u8>>, Vec<u8>) {
    let mut charset = None;
    let mut plural_expr = Vec::new();
    for line in header.split(|&b| b == b'\n') {
        if line.len() >= 13 && line[..13].eq_ignore_ascii_case(b"content-type:") {
            if let Some(pos) = line
                .windows(8)
                .position(|w| w.eq_ignore_ascii_case(b"charset="))
            {
                let rest = &line[pos + 8..];
                let end = rest
                    .iter()
                    .position(|&b| matches!(b, b' ' | b'\t' | b'\r' | b';'))
                    .unwrap_or(rest.len());
                charset = Some(rest[..end].to_vec());
            }
        } else if line.len() >= 13
            && line[..13].eq_ignore_ascii_case(b"plural-forms:")
            && let Some(pos) = line
                .windows(7)
                .position(|w| w.eq_ignore_ascii_case(b"plural="))
        {
            let rest = &line[pos + 7..];
            let end = rest
                .iter()
                .position(|&b| b == b';' || b == b'\r')
                .unwrap_or(rest.len());
            plural_expr = rest[..end].to_vec();
        }
    }
    (charset, plural_expr)
}

fn convert_mo_bytes_to_utf8(bytes: &[u8], charset: Option<&[u8]>) -> Vec<u8> {
    let Some(cs) = charset else {
        return bytes.to_vec();
    };
    if cs.eq_ignore_ascii_case(b"utf-8")
        || cs.eq_ignore_ascii_case(b"utf8")
        || cs.eq_ignore_ascii_case(b"us-ascii")
        || cs.eq_ignore_ascii_case(b"ascii")
    {
        return bytes.to_vec();
    }
    if cs.eq_ignore_ascii_case(b"iso-8859-1")
        || cs.eq_ignore_ascii_case(b"iso_8859-1")
        || cs.eq_ignore_ascii_case(b"latin1")
    {
        let mut out = Vec::with_capacity(bytes.len());
        for &b in bytes {
            if b < 0x80 {
                out.push(b);
            } else {
                out.push(0xC0 | (b >> 6));
                out.push(0x80 | (b & 0x3F));
            }
        }
        return out;
    }
    if let Ok(cs_cstr) = CString::new(cs) {
        let mut vc = Vimconv::new();
        if convert_setup(&mut vc, cs_cstr.as_ptr().cast_mut(), c"utf-8") && vc.vc_type != 0 {
            let mut len = bytes.len();
            if let Some(mut slice) = string_convert(
                &vc,
                bytes.as_ptr().cast::<c_char>().cast_mut(),
                Some(&mut len),
            ) {
                slice.truncate(len);
                return slice;
            }
        }
    }
    bytes.to_vec()
}

fn parse_mo_catalog(path: PathBuf, data: &[u8]) -> Option<MoCatalog> {
    if data.len() < 28 {
        return None;
    }
    let magic = u32::from_le_bytes(data[0..4].try_into().ok()?);
    let read_u32 = |off: usize| -> Option<u32> {
        let b: [u8; 4] = data.get(off..off + 4)?.try_into().ok()?;
        if magic == 0x9504_12de {
            Some(u32::from_le_bytes(b))
        } else if magic == 0xde12_0495 {
            Some(u32::from_be_bytes(b))
        } else {
            None
        }
    };
    let rev = read_u32(4)?;
    if (rev >> 16) > 1 {
        return None;
    }
    let nstrings = read_u32(8)? as usize;
    let orig_tab = read_u32(12)? as usize;
    let trans_tab = read_u32(16)? as usize;

    let mut raw_entries = Vec::with_capacity(nstrings);
    let mut charset = None;
    let mut plural_expr = Vec::new();

    for i in 0..nstrings {
        let o_len = read_u32(orig_tab.checked_add(i * 8)?)? as usize;
        let o_off = read_u32(orig_tab.checked_add(i * 8 + 4)?)? as usize;
        let t_len = read_u32(trans_tab.checked_add(i * 8)?)? as usize;
        let t_off = read_u32(trans_tab.checked_add(i * 8 + 4)?)? as usize;
        let orig = data.get(o_off..o_off.checked_add(o_len)?)?;
        let trans = data.get(t_off..t_off.checked_add(t_len)?)?;
        let msgid = orig.split(|&b| b == 0).next().unwrap_or(b"");
        if msgid.is_empty() {
            let (cs, pe) = parse_mo_header(trans);
            charset = cs;
            plural_expr = pe;
        } else if !trans.is_empty() {
            raw_entries.push((msgid, trans));
        }
    }

    let mut entries = Vec::with_capacity(raw_entries.len());
    for (msgid, trans) in raw_entries {
        let mut forms = Vec::new();
        for form_bytes in trans.split(|&b| b == 0) {
            let utf8 = convert_mo_bytes_to_utf8(form_bytes, charset.as_deref());
            if let Ok(cstr) = CString::new(utf8) {
                forms.push(cstr);
            }
        }
        if !forms.is_empty() {
            entries.push(MoEntry {
                msgid: msgid.into(),
                forms: forms.into_boxed_slice(),
            });
        }
    }
    entries.sort_by(|a, b| a.msgid.cmp(&b.msgid));

    Some(MoCatalog {
        path,
        plural_expr: plural_expr.into_boxed_slice(),
        entries: entries.into_boxed_slice(),
    })
}

fn push_unique_str(vec: &mut Vec<String>, s: String) {
    if !s.is_empty() && !vec.contains(&s) {
        vec.push(s);
    }
}

fn locale_candidates(locale: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in locale.split(':') {
        let part = part.trim();
        if part.is_empty() || matches!(part, "C" | "C.UTF-8" | "C.utf8" | "POSIX") {
            continue;
        }
        let no_mod = part.split('@').next().unwrap_or(part);
        let (base, codeset) = match no_mod.split_once('.') {
            Some((b, c)) => (b, Some(c)),
            None => (no_mod, None),
        };
        let ll = base.split('_').next().unwrap_or(base);
        push_unique_str(&mut out, part.to_string());
        push_unique_str(&mut out, no_mod.to_string());
        if let Some(cs) = codeset
            && (cs.eq_ignore_ascii_case("utf-8") || cs.eq_ignore_ascii_case("utf8"))
        {
            push_unique_str(&mut out, format!("{base}.UTF-8"));
            push_unique_str(&mut out, format!("{ll}.UTF-8"));
        }
        push_unique_str(&mut out, base.to_string());
        push_unique_str(&mut out, ll.to_string());
    }
    out
}

fn current_message_locale() -> Option<String> {
    unsafe {
        let mut buf = [0 as c_char; 256];
        for var in [c"LANGUAGE", c"LC_ALL", c"LC_MESSAGES", c"LANG"] {
            let p = os_getenv_buf(var.as_ptr(), buf.as_mut_ptr(), buf.len());
            if !p.is_null()
                && *p != 0
                && let Ok(s) = CStr::from_ptr(p).to_str()
            {
                return Some(s.to_string());
            }
        }
        #[cfg(not(windows))]
        let p = get_locale_val(LC_MESSAGES);
        #[cfg(windows)]
        let p = get_locale_val(LC_CTYPE);
        if !p.is_null()
            && *p != 0
            && let Ok(s) = CStr::from_ptr(p).to_str()
        {
            return Some(s.to_string());
        }
        None
    }
}

fn ensure_catalog_loaded() -> Option<&'static MoCatalog> {
    unsafe {
        let catalogs = &mut lang_st().catalogs;
        if !lang_st().catalog_valid {
            lang_st().catalog_valid = true;
            lang_st().active_catalog = None;
            if lang_st().locale_dir_len == 0 {
                return None;
            }
            let dir_slice = std::slice::from_raw_parts(
                (&raw const lang_st().locale_dir).cast::<u8>(),
                lang_st().locale_dir_len,
            );
            let dir_str = std::str::from_utf8(dir_slice).ok()?;
            let base_dir = Path::new(dir_str);
            let locale = current_message_locale()?;
            for cand in locale_candidates(&locale) {
                let mo_path = base_dir.join(&cand).join("LC_MESSAGES").join("nvim.mo");
                if let Some(idx) = catalogs.iter().position(|c| c.path == mo_path) {
                    lang_st().active_catalog = Some(idx);
                    break;
                }
                if let Ok(bytes) = std::fs::read(&mo_path)
                    && let Some(cat) = parse_mo_catalog(mo_path, &bytes)
                {
                    let idx = catalogs.len();
                    catalogs.push(cat);
                    lang_st().active_catalog = Some(idx);
                    break;
                }
            }
        }
        lang_st().active_catalog.and_then(|idx| catalogs.get(idx))
    }
}

pub(crate) fn c_gettext(msgid: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let msgid = msgid.into_c_char_ptr();
    if msgid.is_null() || unsafe { *msgid == 0 } {
        return msgid.cast_mut();
    }
    {
        let Some(cat) = ensure_catalog_loaded() else {
            return msgid.cast_mut();
        };
        let key = crate::memory::cstr_as_bytes(msgid);
        if let Ok(idx) = cat.entries.binary_search_by(|e| e.msgid.as_ref().cmp(key))
            && let Some(first) = cat.entries[idx].forms.first()
        {
            return first.as_ptr().cast_mut();
        }
        msgid.cast_mut()
    }
}

pub(crate) fn c_ngettext(
    msgid1: impl crate::strings::IntoCCharPtr,
    msgid2: impl crate::strings::IntoCCharPtr,
    n: u64,
) -> *mut c_char {
    let msgid1 = msgid1.into_c_char_ptr();
    let msgid2 = msgid2.into_c_char_ptr();
    let fallback = if n == 1 {
        msgid1.cast_mut()
    } else {
        msgid2.cast_mut()
    };
    if msgid1.is_null() || unsafe { *msgid1 == 0 } {
        return fallback;
    }
    {
        let Some(cat) = ensure_catalog_loaded() else {
            return fallback;
        };
        let key = crate::memory::cstr_as_bytes(msgid1);
        if let Ok(idx) = cat.entries.binary_search_by(|e| e.msgid.as_ref().cmp(key)) {
            let forms = &cat.entries[idx].forms;
            let form_idx = eval_plural_expr(&cat.plural_expr, n);
            if let Some(form) = forms.get(form_idx).or_else(|| forms.first()) {
                return form.as_ptr().cast_mut();
            }
        }
        fallback
    }
}

#[inline]
pub fn bind_textdomain_codeset(
    _domainname: *const c_char,
    codeset: impl crate::strings::IntoCCharPtr,
) -> *mut c_char {
    let codeset = codeset.into_c_char_ptr();
    codeset.cast_mut()
}

use crate::charset::NUL;
#[cfg(unix)]
use crate::diff::K_SHELL_OPT_SILENT;
use crate::os::fs::MAXPATHL;

#[inline]
fn get_locale_val(what: c_int) -> *mut c_char {
    unsafe { setlocale(what, ptr::null()) }
}

#[inline]
fn is_valid_mess_lang(lang: impl crate::strings::IntoCCharPtr) -> bool {
    let lang = lang.into_c_char_ptr();
    if lang.is_null() {
        return false;
    }
    (crate::memory::cstr_first_byte(lang)).is_ascii_alphabetic()
        && (crate::memory::cstr_byte_at(lang, 1)).is_ascii_alphabetic()
}

/// Obtain the current messages language. Used to set the default for `'helplang'`.
pub fn get_mess_lang() -> *mut c_char {
    #[cfg(not(windows))]
    let p = get_locale_val(LC_MESSAGES);
    #[cfg(windows)]
    let p = get_locale_val(LC_COLLATE);

    if is_valid_mess_lang(p) {
        p
    } else {
        ptr::null_mut()
    }
}

fn get_mess_env(
    #[cfg_attr(not(windows), allow(unused_variables))] buf: &mut [c_char; 64],
) -> *mut c_char {
    #[cfg(not(windows))]
    {
        get_locale_val(LC_MESSAGES)
    }
    #[cfg(windows)]
    {
        let mut p = os_getenv_buf(c"LC_ALL", buf.as_mut_ptr(), buf.len());
        if !p.is_null() {
            return p;
        }
        p = os_getenv_buf(c"LC_MESSAGES", buf.as_mut_ptr(), buf.len());
        if !p.is_null() {
            return p;
        }
        p = os_getenv_buf(c"LANG", buf.as_mut_ptr(), buf.len());
        if !p.is_null() && (crate::memory::cstr_first_byte(p)).is_ascii_digit() {
            p = ptr::null_mut();
        }
        if p.is_null() {
            p = get_locale_val(LC_CTYPE);
        }
        p
    }
}

/// Set the `v:lang`, `v:lc_time`, `v:collate`, and `v:ctype` variables.
pub fn set_lang_var() {
    let mut buf = [0 as c_char; 64];
    let mut loc: *const c_char = get_locale_val(LC_CTYPE);
    set_vim_var_string(VV_CTYPE, loc, -1);

    loc = get_mess_env(&mut buf);
    set_vim_var_string(VV_LANG, loc, -1);

    loc = get_locale_val(LC_TIME);
    set_vim_var_string(VV_LC_TIME, loc, -1);

    loc = get_locale_val(LC_COLLATE);
    set_vim_var_string(VV_COLLATE, loc, -1);
}

/// Setup to use the current locale.
pub fn init_locale() {
    unsafe {
        let _ = setlocale(LC_ALL, c"".as_ptr());
        let _ = setlocale(LC_NUMERIC, c"C".as_ptr());

        let mut localepath = [0 as c_char; MAXPATHL];
        xstrlcpy(
            localepath.as_mut_ptr(),
            get_vim_var_str(VV_PROGPATH),
            MAXPATHL,
        );
        let mut tail = path_tail_with_sep(localepath.as_mut_ptr());
        *tail = NUL;
        tail = path_tail(localepath.as_ptr());
        let offset = (tail as usize) - (localepath.as_ptr() as usize);
        xstrlcpy(tail, c"share/locale", MAXPATHL - offset);

        if HAVE_LIBINTL {
            let bytes = crate::memory::cstr_as_bytes(localepath.as_ptr());
            let len = bytes.len().min(MAXPATHL);
            lang_st().locale_dir[..len].copy_from_slice(&bytes[..len]);
            lang_st().locale_dir_len = len;
            lang_st().catalog_valid = false;
        }

        if has_time_fd() {
            time_msg(c"locale set", None);
        }
    }
}

/// `":language"`: Set the language (locale).
pub fn ex_language(eap: &mut Exarg) {
    unsafe {
        let mut what = LC_ALL;
        let mut whatstr: *const c_char = c"".as_ptr();
        let mut name = eap.arg;

        let p = skiptowhite(eap.arg);
        let arg_len = (p as usize) - (eap.arg as usize);
        if (crate::memory::cstr_is_empty(p)
            || matches!(crate::memory::cstr_first_byte(p), b' ' | b'\t'))
            && arg_len >= 3
        {
            if mb_strnicmp(eap.arg, c"messages", arg_len) == 0 {
                what = VIM_LC_MESSAGES;
                name = skipwhite(p);
                whatstr = c"messages ".as_ptr();
            } else if mb_strnicmp(eap.arg, c"ctype", arg_len) == 0 {
                what = LC_CTYPE;
                name = skipwhite(p);
                whatstr = c"ctype ".as_ptr();
            } else if mb_strnicmp(eap.arg, c"time", arg_len) == 0 {
                what = LC_TIME;
                name = skipwhite(p);
                whatstr = c"time ".as_ptr();
            } else if mb_strnicmp(eap.arg, c"collate", arg_len) == 0 {
                what = LC_COLLATE;
                name = skipwhite(p);
                whatstr = c"collate ".as_ptr();
            }
        }

        if crate::memory::cstr_is_empty(name) {
            let mut mess_buf = [0 as c_char; 64];
            let mut p_val = if what == VIM_LC_MESSAGES {
                get_mess_env(&mut mess_buf)
            } else {
                setlocale(what, ptr::null())
            };
            if crate::memory::cstr_is_empty(p_val) {
                p_val = c"Unknown".as_ptr().cast_mut();
            }
            let _ = smsg!(0, gettext(c"Current %slanguage: \"%s\""), whatstr, p_val,);
        } else {
            #[cfg(windows)]
            let loc = if what == VIM_LC_MESSAGES {
                c"".as_ptr().cast_mut()
            } else {
                let l = setlocale(what, name);
                let _ = setlocale(LC_NUMERIC, c"C".as_ptr());
                l
            };
            #[cfg(not(windows))]
            let loc = {
                let l = setlocale(what, name);
                let _ = setlocale(LC_NUMERIC, c"C".as_ptr());
                l
            };

            if loc.is_null() {
                let _ = semsg!(gettext(c"E197: Cannot set language to \"%s\""), name,);
            } else {
                lang_st().catalog_valid = false;
                let _ = os_setenv(c"LC_ALL", c"", 1);

                if what != LC_TIME && what != LC_COLLATE {
                    if what == LC_ALL {
                        let _ = os_setenv(c"LANG", name, 1);
                        let _ = os_setenv(c"LANGUAGE", c"", 1);
                    }
                    if what != LC_CTYPE {
                        let _ = os_setenv(c"LC_MESSAGES", name, 1);
                        set_helplang_default(name);
                    }
                }

                set_lang_var();
                maketitle();
            }
        }

        let _ = buf_init_isk_chartab(curbuf_mut());
        let _ = init_isf_chartab();
        let _ = init_isi_chartab();
    }
}

#[cfg(not(windows))]
fn find_locales() -> Vec<CString> {
    let Some(locale_a) = get_cmd_output(c"locale -a", ptr::null_mut(), K_SHELL_OPT_SILENT) else {
        return Vec::new();
    };
    locale_a
        .as_bytes()
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| CString::new(line).ok())
        .collect()
}

fn init_locales() {
    #[cfg(not(windows))]
    {
        if lang_st().did_init_locales {
            return;
        }
        lang_st().did_init_locales = true;
        lang_st().locales = find_locales();
    }
}

pub fn free_locales() {
    lang_st().locales = Vec::new();
}

/// Function given to `ExpandGeneric()` to obtain the possible arguments of `":language"`.
pub fn get_lang_arg(_xp: &mut ExpandCtx, idx: c_int) -> *mut c_char {
    match idx {
        0 => c"messages".as_ptr().cast_mut(),
        1 => c"ctype".as_ptr().cast_mut(),
        2 => c"time".as_ptr().cast_mut(),
        3 => c"collate".as_ptr().cast_mut(),
        _ => get_locales(_xp, idx - 4),
    }
}

/// Function given to `ExpandGeneric()` to obtain the available locales.
pub fn get_locales(_xp: &mut ExpandCtx, idx: c_int) -> *mut c_char {
    if idx < 0 {
        return ptr::null_mut();
    }
    {
        init_locales();
        let locales = &lang_st().locales;
        locales
            .get(idx as usize)
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }
}

pub fn lang_init() {
    #[cfg(target_os = "macos")]
    unsafe {
        if !os_env_exists(c"LANG", true) {
            let _ = os_setenv(c"LANG", c"C.UTF-8", 1);
            let _ = setlocale(LC_ALL, c"C.UTF-8");
            let _ = setlocale(LC_NUMERIC, c"C");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_mess_lang_and_get_lang_arg_work() {
        {
            assert!(!is_valid_mess_lang(ptr::null()));
            assert!(!is_valid_mess_lang(c"C"));
            assert!(!is_valid_mess_lang(c"C.UTF-8"));
            assert!(is_valid_mess_lang(c"en_US.UTF-8"));

            assert_eq!(
                crate::memory::cstr_as_bytes(get_lang_arg(
                    &mut crate::os::env::ExpandCtx::new(),
                    0
                )),
                b"messages"
            );
            assert_eq!(
                crate::memory::cstr_as_bytes(get_lang_arg(
                    &mut crate::os::env::ExpandCtx::new(),
                    3
                )),
                b"collate"
            );

            assert_eq!(eval_plural_expr(b"(n != 1)", 1), 0);
            assert_eq!(eval_plural_expr(b"(n != 1)", 2), 1);
            let slavic =
                b"n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2";
            assert_eq!(eval_plural_expr(slavic, 1), 0);
            assert_eq!(eval_plural_expr(slavic, 2), 1);
            assert_eq!(eval_plural_expr(slavic, 5), 2);
            assert_eq!(eval_plural_expr(slavic, 11), 2);
            assert_eq!(eval_plural_expr(slavic, 21), 0);

            let cands = locale_candidates("ja_JP.UTF-8");
            assert!(cands.iter().any(|c| c == "ja"));
        }
    }
}
