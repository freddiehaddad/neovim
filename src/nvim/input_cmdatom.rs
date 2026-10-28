//! Input engine command-atom policy layer (`src/nvim/input_cmdatom.c`).

use crate::api::private::helpers::{
    api_free_dict, array_obj, boolean_obj, cstr_to_obj, cstr_to_string, dict_obj, integer_obj,
};
use crate::api::private::validate::{Array, Dict, KeyValuePair, Object};
use crate::autocmd::{AUGROUP_ALL, EVENT_CMDATOM, aucmd_defer, has_event};
use crate::buffer::{buf_T, buf_get_changedtick, bufref_T, bufref_valid, set_bufref};
use crate::charset::is_special;
use crate::cursor::{equalpos, pos_T};
use crate::eval::typval::varnumber_T;
use crate::eval::vars::{VV_OP, get_vim_var_str};
use crate::extmark::{TriState, kFalse, kNone, kTrue};
use crate::globals::*;
use crate::input::{
    NUL, prep_redo, prep_redo_visual, redo_append_lit, redo_append_spec, redo_append_str,
    redo_keys, redo_new, redo_spec, sb_add_char, sb_add_lit, sb_add_spec, stuff_empty,
    typebuf_typed,
};
use crate::insert::get_last_insert_save;
use crate::keycodes::{
    CAR, DEL, K_BS, K_COMMAND, K_DEL, K_DOWN, K_END, K_HOME, K_IGNORE, K_LEFT, K_LEFTMOUSE,
    K_LEFTMOUSE_NM, K_MIDDLEMOUSE, K_MOUSEDOWN, K_MOUSELEFT, K_MOUSERIGHT, K_MOUSEUP, K_RIGHT,
    K_RIGHTMOUSE, K_S_LEFT, K_S_RIGHT, K_SPECIAL, K_UP, K_X1MOUSE, K_X2MOUSE, NL,
    get_special_key_name, termcap2key,
};
use crate::mbyte::utf_char2bytes;
use crate::mcursor::{
    mc_buf_has_cursors, mc_clock_edge, mc_following, mc_ins_cascade_start, mc_ins_cascaded,
    mc_ins_commit, mc_mark_at, mc_replaying, mc_vsel_clear, mc_vsel_refresh,
};
use crate::memory::{cstr_from_bytes, cstr_to_cstring};
use crate::normal::{
    NV_JUMP, NV_LANG, NV_MOTION, NV_NCH_ARG, REPLACE_CR_NCHAR, VisualState, nv_is,
};
use crate::ops::{
    OP_APPEND, OP_CHANGE, OP_INSERT, OP_NOP, OP_NR_ADD, OP_NR_SUB, OP_REPLACE, get_extra_op_char,
    get_op_char, op_redoable, op_self_select,
};
use crate::option_vars::*;
use crate::register::reg_max_ts;
use crate::state::{MODE_NORMAL, MODE_OP_PENDING, get_real_state};
use crate::strings::{vim_snprintf, vim_strchr};
use crate::textformat::oparg_T;
use crate::viml::parser::expressions::ESC;
use crate::window::{win_T, win_valid};
use std::ffi::{CStr, c_char, c_int, c_long, c_uint};
use std::ptr;

pub type CmdAtomVec = Vec<CmdAtom>;
use crate::os::time::Timestamp;

use crate::api::ui::MAX_SCHAR_SIZE;
use crate::cmdexpand::MB_MAXBYTES;
use crate::ex_cmds::{Ctrl_E, Ctrl_Y};
use crate::ex_getln::{
    Ctrl_B, Ctrl_D, Ctrl_F, Ctrl_G, Ctrl_R, Ctrl_T, Ctrl_U, Ctrl_W, K_EVENT, K_LUA,
};
use crate::mapping::MAXMAPLEN;
use crate::regexp::{Ctrl_H, Ctrl_V, NUMBUFLEN};

#[derive(Clone)]
pub struct cmdarg_T {
    pub oap: *mut oparg_T,
    pub cmdchar: c_int,
    pub nchar: c_int,
    pub nchar_composing: [c_char; MAX_SCHAR_SIZE],
    pub nchar_len: c_int,
    pub extra_char: c_int,
    pub opcount: c_int,
    pub count0: c_int,
    pub count1: c_int,
    pub arg: c_int,
    pub retval: c_int,
    pub searchbuf: Option<std::ffi::CString>,
}

impl cmdarg_T {
    pub const fn new() -> Self {
        Self {
            oap: ptr::null_mut(),
            cmdchar: 0,
            nchar: 0,
            nchar_composing: [0; MAX_SCHAR_SIZE],
            nchar_len: 0,
            extra_char: 0,
            opcount: 0,
            count0: 0,
            count1: 0,
            arg: 0,
            retval: 0,
            searchbuf: None,
        }
    }

    #[inline]
    pub fn searchbuf_ptr(&self) -> *mut c_char {
        self.searchbuf
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }

    #[inline]
    pub fn oap_mut(&self) -> &'static mut oparg_T {
        unsafe { &mut *self.oap }
    }
}

impl Default for cmdarg_T {
    fn default() -> Self {
        Self::new()
    }
}

const kCpoRedo: c_int = b'r' as c_int;

pub const kAComp: c_int = 0;
pub const kAExcmd: c_int = 1;
pub const kAInsert: c_int = 2;
pub const kAInsertSpan: c_int = 3;
pub const kAJump: c_int = 4;
pub const kAMotion: c_int = 5;
pub const kAMouse: c_int = 6;
pub const kANormal: c_int = 7;
pub const kAOperator: c_int = 8;
pub const kAScroll: c_int = 9;
pub const kAVisual: c_int = 10;
pub const kAVisualSpan: c_int = 11;
pub type CmdAtomType = c_int;

pub const kVInsNone: c_int = 0;
pub const kVInsKeys: c_int = 1;
pub const kVInsMotion: c_int = 2;
pub const kVInsOther: c_int = 3;
pub type VisualIns = c_int;

pub const kKeyOpaque: c_uint = 1 << 0;
pub const kKeySynthetic: c_uint = 1 << 1;
pub const kKeyPayload: c_uint = 1 << 2;
pub const kKeyScrollMove: c_uint = 1 << 3;
pub const kKeyScrollView: c_uint = 1 << 4;
pub const kKeyJump: c_uint = 1 << 5;
pub const kKeyMotion: c_uint = 1 << 6;
pub const kKeyInsFlush: c_uint = 1 << 7;
pub const kKeyMouse: c_uint = 1 << 8;

const kVatomNone: u32 = 0;
const kVatomTyped: u32 = 1;
const kVatomFed: u32 = 2;
const kVatomVoid: u32 = 4;

const TYPE_NAMES: [&CStr; 12] = [
    c"mapping",
    c"excmd",
    c"insert",
    c"insert",
    c"jump",
    c"motion",
    c"mouse",
    c"normal",
    c"operator",
    c"scroll",
    c"visual",
    c"visual",
];

#[repr(C)]
#[derive(Clone)]
pub struct CmdSpec {
    pub count: c_long,
    pub regname: c_int,
    pub op: c_int,
    pub op_extra: c_int,
    pub motion_force: c_int,
    pub cmd: c_int,
    pub cmd2: c_int,
    pub cmdarg: c_int,
    pub body: Vec<c_char>,
}

impl CmdSpec {
    pub const fn new() -> Self {
        Self {
            count: 0,
            regname: 0,
            op: 0,
            op_extra: 0,
            motion_force: 0,
            cmd: 0,
            cmd2: 0,
            cmdarg: 0,
            body: Vec::new(),
        }
    }

    pub const fn without_body(&self) -> Self {
        Self {
            count: self.count,
            regname: self.regname,
            op: self.op,
            op_extra: self.op_extra,
            motion_force: self.motion_force,
            cmd: self.cmd,
            cmd2: self.cmd2,
            cmdarg: self.cmdarg,
            body: Vec::new(),
        }
    }
}

impl Default for CmdSpec {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CmdOrigin {
    pub buf: bufref_T,
    pub win: *const win_T,
    pub pos: pos_T,
    pub mcursor: u32,
    pub tick: varnumber_T,
    pub maptick: c_int,
}

impl CmdOrigin {
    pub const fn new() -> Self {
        Self {
            buf: bufref_T {
                br_buf: ptr::null_mut(),
                br_fnum: 0,
                br_buf_free_count: 0,
            },
            win: ptr::null(),
            pos: pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            mcursor: 0,
            tick: 0,
            maptick: 0,
        }
    }
}

impl Default for CmdOrigin {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct InsSession {
    pub origin: CmdOrigin,
    pub typed: bool,
    pub vis: VisualIns,
    pub vsel: Option<std::ffi::CString>,
}

#[repr(C)]
pub struct CmdAtom {
    pub r#type: CmdAtomType,
    pub spec: CmdSpec,
    pub origin: CmdOrigin,
    pub atoms: CmdAtomVec,
    pub keys: Option<std::ffi::CString>,
    pub text: Option<std::ffi::CString>,
    pub lhs: Option<std::ffi::CString>,
    pub undoseq: c_int,
    pub changed: bool,
    pub moved: bool,
    pub remap: bool,
    pub cascaded: bool,
}

impl CmdAtom {
    pub const fn new() -> Self {
        Self {
            r#type: kAComp,
            spec: CmdSpec::new(),
            origin: CmdOrigin::new(),
            atoms: Vec::new(),
            keys: None,
            text: None,
            lhs: None,
            undoseq: 0,
            changed: false,
            moved: false,
            remap: false,
            cascaded: false,
        }
    }
}

impl Default for CmdAtom {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct CmdFrame {
    pub origin: CmdOrigin,
    pub staged: CmdAtom,
    pub visual: VisualState,
    pub keytyped: bool,
    pub keyclass: c_uint,
    pub ex_normal: c_int,
    pub captures: u64,
    pub global_ops: u64,
    pub beeps: u64,
    pub id: u64,
    pub follow: bool,
    pub consumers: bool,
    pub reg_ts: Timestamp,
    pub payload_start: usize,
    pub payload_end: usize,
    pub redo_frame: u64,
    pub cmdline: Option<std::ffi::CString>,
}

struct CompositeState {
    active: bool,
    open: bool,
    origin: CmdOrigin,
    follow: TriState,
    atoms: CmdAtomVec,
    lhs: [c_char; MAXMAPLEN + 4],
    queued: bool,
    lossy: bool,
    r#macro: bool,
    frame: u64,
}

struct VatomStateStruct {
    atoms: CmdAtomVec,
    state: u32,
    origin: CmdOrigin,
    frame: u64,
    done: usize,
}

struct TypedState {
    keys: Vec<u8>,
    map_start: usize,
}

pub(crate) struct CmdAtomModuleState {
    pub(crate) g_atoms: CmdAtomVec,
    atom_captures: u64,
    global_ops: u64,
    atom_suppressed: bool,
    map_edit: bool,
    frame_id: u64,
    cmd_frames: Vec<CmdFrame>,
    composite: CompositeState,
    vatom: VatomStateStruct,
    typed: TypedState,
}

impl CmdAtomModuleState {
    const fn new() -> Self {
        Self {
            g_atoms: Vec::new(),
            atom_captures: 0,
            global_ops: 0,
            atom_suppressed: false,
            map_edit: false,
            frame_id: 0,
            cmd_frames: Vec::new(),
            composite: CompositeState { active: false, open: false, origin: CmdOrigin::new(), follow: kNone, atoms: Vec::new(), lhs: [0; MAXMAPLEN + 4], queued: false, lossy: false, r#macro: false, frame: 0 },
            vatom: VatomStateStruct { atoms: Vec::new(), state: kVatomNone, origin: CmdOrigin::new(), frame: 0, done: 0 },
            typed: TypedState { keys: Vec::new(), map_start: 0 },
        }
    }
}

static ATOM_STATE: GlobalState<CmdAtomModuleState> = GlobalState::new(CmdAtomModuleState::new());

#[inline]
pub(crate) fn atom_st() -> &'static mut CmdAtomModuleState {
    unsafe { &mut *ATOM_STATE.as_ptr() }
}

#[inline]
pub fn sb_concat(sb: &mut Vec<c_char>, s: impl crate::strings::IntoCCharPtr) {
    let s = s.into_c_char_ptr();
    unsafe {
        if !s.is_null() {
            let len = crate::memory::cstr_len(s);
            if len > 0 {
                sb.extend_from_slice(std::slice::from_raw_parts(s, len));
            }
        }
    }
}

#[inline]
fn dict_put(d: &mut Dict, key: &CStr, value: Object) {
    d.push(KeyValuePair {
            key: cstr_to_string(key.as_ptr()),
            value,
        });
}

use crate::api::private::helpers::array_add;

#[inline]
fn cur_frame_ptr() -> *mut CmdFrame {
    unsafe {
        let frames = &raw mut atom_st().cmd_frames;
        (*frames).last_mut().map_or(ptr::null_mut(), ptr::from_mut)
    }
}

#[inline]
fn root_frame() -> *mut CmdFrame {
    unsafe {
        let frames = &raw mut atom_st().cmd_frames;
        assert!(!(*frames).is_empty());
        (*frames).as_mut_ptr()
    }
}

#[inline]
fn is_child_frame() -> bool {
    unsafe {
        let frames = &raw const atom_st().cmd_frames;
        if (*frames).len() <= 1 {
            return false;
        }
        let root = &(*frames).as_slice()[0];
        let cur = (*frames).last().unwrap();
        let fed = cur.ex_normal > root.ex_normal;
        fed && (root.keyclass & kKeySynthetic) == 0
    }
}

pub fn atom_free_all() {
    unsafe {
        atom_st().g_atoms = Vec::new();
        let frames = &raw mut atom_st().cmd_frames;
        for frame in (*frames).iter_mut().rev() {
            frame.staged = CmdAtom::new();
            frame.cmdline = None;
        }
        atom_composite_abort();
        let comp_atoms = &raw mut atom_st().composite.atoms;
        *comp_atoms = Vec::new();
        let typed_keys = &raw mut atom_st().typed.keys;
        *typed_keys = Vec::new();
        atom_st().vatom.atoms = Vec::new();
    }
}

pub fn atom_cmd_spec(cap: &cmdarg_T) -> CmdSpec {
     {
        let operand = nv_is(cap.cmdchar, NV_LANG | NV_NCH_ARG);
        CmdSpec {
            regname: cap.oap_mut().regname,
            count: c_long::from(cap.count0),
            cmd: cap.cmdchar,
            cmd2: if operand { NUL } else { cap.nchar },
            cmdarg: if operand {
                cap.nchar
            } else {
                cap.extra_char
            },
            ..CmdSpec::default()
        }
    }
}

fn atom_origin() -> CmdOrigin {
     {
        let mut origin = CmdOrigin {
            win: curwin_ptr(),
            pos: curwin_mut().w_cursor,
            tick: buf_get_changedtick(curbuf_mut()),
            maptick: g_st().maptick,
            mcursor: mc_mark_at(curbuf_mut(), curwin_mut().w_cursor, 0),
            ..CmdOrigin::default()
        };
        set_bufref(&mut origin.buf, g_st().curbuf);
        origin
    }
}

#[inline]
fn atom_origin_changed(origin: CmdOrigin) -> bool {
    unsafe {
        bufref_valid(Some(&origin.buf))
            && buf_get_changedtick(&*origin.buf.br_buf) != origin.tick
    }
}

#[inline]
fn atom_origin_moved(origin: CmdOrigin) -> bool {
    unsafe {
        bufref_valid(Some(&origin.buf))
            && win_valid(origin.win)
            && (*origin.win).w_buffer == origin.buf.br_buf
            && !equalpos(origin.pos, (*origin.win).w_cursor)
    }
}

#[inline]
fn atom_origin_undoseq(origin: CmdOrigin) -> c_int {
    unsafe {
        if bufref_valid(Some(&origin.buf)) {
            (*origin.buf.br_buf).b_u_seq_cur
        } else {
            0
        }
    }
}

pub fn atom_origin_pos(buf: &mut buf_T) -> pos_T {
     {
        let o = atom_st().composite.origin;
        if atom_st().composite.open
            && o.buf.br_buf == (&raw mut *buf)
            && bufref_valid(Some(&o.buf))
            && buf_get_changedtick(buf) == o.tick
        {
            return o.pos;
        }
        curwin_mut().w_cursor
    }
}

fn atom_redo_keys(spec: &CmdSpec) -> std::ffi::CString {
    assert!(!mc_replaying());
        redo_keys(spec).expect("atom_redo_keys must be non-empty")
}

fn atom_from_redo(r#type: CmdAtomType) -> CmdAtom {
    let keys_cstr = redo_keys(ptr::null());
        let mut atom = CmdAtom::new();
        atom.r#type = r#type;
        atom.spec = redo_spec();
        atom.keys = keys_cstr;
        atom
}

fn atom_from_spec(r#type: CmdAtomType, spec: CmdSpec) -> CmdAtom {
    let keys_cstr = atom_redo_keys(&spec);
        let mut atom = CmdAtom::new();
        atom.r#type = r#type;
        atom.spec = spec;
        atom.keys = Some(keys_cstr);
        atom
}

fn atom_from_cmdline(
    r#type: CmdAtomType,
    ca: &mut cmdarg_T,
    cmdline: *const c_char,
    vis: bool,
) -> CmdAtom {
     {
        let mut sb: Vec<c_char> = Vec::new();
        if ca.cmdchar != b':' as c_int && ca.count0 != 0 {
            let mut num_buf = [0 as c_char; 32];
            let n = vim_snprintf!(
                num_buf.as_mut_ptr(),
                num_buf.len(),
                c"%d",
                ca.count0,
            );
            if n > 0 {
                sb.extend_from_slice(&num_buf[..n as usize]);
            }
        }
        sb_add_char(&mut sb, ca.cmdchar);
        if vis && ca.cmdchar == b':' as c_int {
            sb_add_char(&mut sb, Ctrl_U);
        }
        if is_special(ca.cmdchar) {
            sb_add_spec(&mut sb, cmdline);
        } else {
            sb_add_lit(&mut sb, cmdline, -1);
        }
        sb_add_char(&mut sb, NL);
        let sb_u8 = crate::memory::cstr_slice(sb.as_ptr(), sb.len());
        let mut atom = CmdAtom::new();
        atom.r#type = r#type;
        atom.spec = CmdSpec {
            count: c_long::from(ca.count0),
            cmd: ca.cmdchar,
            ..CmdSpec::default()
        };
        atom.keys = Some(std::ffi::CString::new(sb_u8).unwrap());
        atom.text = Some(crate::memory::cstr_to_cstring(cmdline));
        atom
    }
}

fn atom_from_frame(ca: &mut cmdarg_T, frame: &mut CmdFrame) -> CmdAtom {
    unsafe {
        let root = root_frame();
        let mut atom = CmdAtom::default();
        if let Some(ref cmdline) = (*root).cmdline
            && (ca.cmdchar == b':' as c_int || ca.cmdchar == K_COMMAND)
        {
            atom = atom_from_cmdline(kAExcmd, ca, cmdline.as_ptr(), frame.visual.active);
        } else if ca.cmdchar == K_LUA {
            let mut lua_id = [0 as c_char; NUMBUFLEN];
            vim_snprintf!(
                lua_id.as_mut_ptr(),
                lua_id.len(),
                c"%d",
                g_st().repeat_luamap,
            );
            atom = atom_from_cmdline(kANormal, ca, lua_id.as_ptr(), frame.visual.active);
        }
        atom_payload_append(&mut atom,  frame);
        atom
    }
}

fn atoms_concat_keys(atoms: &[CmdAtom], from: usize) -> std::ffi::CString {
     {
        let mut keys: Vec<c_char> = Vec::new();
        for atom in &atoms[from..] {
            if let Some(ref k) = atom.keys {
                sb_concat(&mut keys, k.as_ptr());
            }
        }
        let bytes = crate::memory::cstr_slice(keys.as_ptr(), keys.len());
        cstr_from_bytes(bytes)
    }
}

fn atom_key_name(c: c_int) -> std::ffi::CString {
    if c == NUL || c == K_LUA {
            return std::ffi::CString::default();
        }
        if is_special(c) || c < b' ' as c_int {
            return cstr_to_cstring(get_special_key_name(c, 0));
        }
        if c == DEL {
            return c"<Del>".to_owned();
        }
        let mut buf = [0 as c_char; MB_MAXBYTES + 1];
        let n = utf_char2bytes(c, buf.as_mut_ptr()) as usize;
        buf[n] = 0;
        cstr_to_cstring(buf.as_ptr())
}

fn atom_dict(atom: &CmdAtom) -> Dict {
    unsafe {
        let spec = &atom.spec;
        let regname: [c_char; 2] = [spec.regname as c_char, 0];
        let force_buf: [c_char; 2] = [spec.motion_force as c_char, 0];
        let force: *const c_char = if spec.motion_force == Ctrl_V {
            c"<C-V>".as_ptr()
        } else {
            force_buf.as_ptr()
        };

        let mut op_bytes = atom_key_name(spec.op).into_bytes();
        if spec.op_extra != NUL {
            op_bytes.extend_from_slice(atom_key_name(spec.op_extra).as_bytes());
        }
        let op = cstr_from_bytes(&op_bytes);

        let mut cmd_bytes = atom_key_name(spec.cmd).into_bytes();
        if spec.cmd2 != NUL {
            cmd_bytes.extend_from_slice(atom_key_name(spec.cmd2).as_bytes());
        }
        let cmd = cstr_from_bytes(&cmd_bytes);

        let mut d = Dict {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        let cmdarg = atom_key_name(spec.cmdarg);
        if !cmdarg.as_bytes().is_empty() {
            dict_put(&mut d, c"cmdarg", cstr_to_obj(cmdarg.as_ptr()));
        }
        dict_put(&mut d, c"changed", boolean_obj(atom.changed));
        if !cmd.as_bytes().is_empty() {
            dict_put(&mut d, c"cmd", cstr_to_obj(cmd.as_ptr()));
        }
        if spec.count > 0 {
            dict_put(&mut d, c"count", integer_obj(spec.count as _));
        }
        if atom.origin.pos.lnum > 0 {
            dict_put(&mut d, c"moved", boolean_obj(atom.moved));
            let mut pos = Array {
                items: ptr::null_mut(),
                size: 0,
                capacity: 0,
            };
            array_add(&mut pos, integer_obj(i64::from(atom.origin.pos.lnum)));
            array_add(&mut pos, integer_obj(i64::from(atom.origin.pos.col)));
            dict_put(&mut d, c"pos", array_obj(pos));
            dict_put(&mut d, c"undoseq", integer_obj(i64::from(atom.undoseq)));
        }
        let keys: *const c_char = atom.keys.as_ref().map_or(c"".as_ptr(), |s| s.as_ptr());
        if !atom.remap {
            dict_put(&mut d, c"keys", cstr_to_obj(keys));
        }
        let lhs_ptr = match &atom.lhs {
            Some(lhs) if !lhs.as_bytes().is_empty() => lhs.as_ptr(),
            _ => keys,
        };
        dict_put(&mut d, c"lhs", cstr_to_obj(lhs_ptr));
        if *force != 0 {
            dict_put(&mut d, c"motionforce", cstr_to_obj(force));
        }
        if !op.as_bytes().is_empty() {
            dict_put(&mut d, c"operator", cstr_to_obj(op.as_ptr()));
        }
        if spec.regname != 0 {
            dict_put(&mut d, c"reg", cstr_to_obj(regname.as_ptr()));
        }
        if let Some(ref text) = atom.text
            && !text.as_bytes().is_empty()
        {
            dict_put(&mut d, c"text", cstr_to_obj(text.as_ptr()));
        }
        dict_put(
            &mut d,
            c"type",
            cstr_to_obj(TYPE_NAMES[atom.r#type as usize].as_ptr()),
        );
        d
    }
}

fn atom_emit(atom: &CmdAtom) {
    unsafe {
        if !has_event(EVENT_CMDATOM) {
            return;
        }
        let mut data = atom_dict( atom);
        if !atom.atoms.is_empty() {
            let mut atoms = Array {
                items: ptr::null_mut(),
                size: 0,
                capacity: 0,
            };
            for sub in &atom.atoms {
                array_add(&mut atoms, dict_obj(atom_dict( sub)));
            }
            dict_put(&mut data, c"atoms", array_obj(atoms));
        }
        let buf = if !atom.origin.buf.br_buf.is_null() {
            atom.origin.buf.br_buf
        } else {
            g_st().curbuf
        };
        let mut obj = dict_obj(data);
        aucmd_defer(
            EVENT_CMDATOM,
            TYPE_NAMES[atom.r#type as usize].as_ptr().cast_mut(),
            ptr::null_mut(),
            AUGROUP_ALL,
            &mut *buf,
            Some(&mut obj),
        );
        api_free_dict(data);
    }
}

pub fn atom_push_raw(cascade: bool,atom: &mut CmdAtom) {
    unsafe {
        assert!(atom.keys.is_some());
        if !atom.origin.buf.br_buf.is_null() {
            atom.changed = atom_origin_changed(atom.origin);
            atom.moved = atom_origin_moved(atom.origin);
            atom.undoseq = atom_origin_undoseq(atom.origin);
        }
        let cur_frame = cur_frame_ptr();
        let collect =
            atom_st().composite.active && (cur_frame.is_null() || (*cur_frame).id != atom_st().composite.frame);
        if atom_visual_pending() {
            if (atom_st().vatom.state & kVatomTyped) != 0 {
                atom_st().atom_captures += 1;
            }
            if collect {
                atom_st().composite.queued = true;
            }
            let v_atoms = &raw mut atom_st().vatom.atoms;
            (*v_atoms).push(std::mem::take(atom));
            atom_st().vatom.frame = if !cur_frame.is_null() {
                (*cur_frame).id
            } else {
                0
            };
            return;
        }
        atom_st().atom_captures += 1;
        if atom.r#type == kAVisual
            && let Some(last) = atom.atoms.last_mut()
            && last.r#type == kAOperator
        {
            last.changed = atom.changed;
        }
        if cascade {
            let copy = CmdAtom {
                r#type: atom.r#type,
                spec: atom.spec.without_body(),
                origin: atom.origin,
                atoms: Vec::new(),
                keys: atom.keys.clone(),
                text: None,
                lhs: None,
                undoseq: atom.undoseq,
                changed: atom.changed,
                moved: atom.moved,
                remap: atom.remap,
                cascaded: atom.cascaded,
            };
            let ga = &raw mut atom_st().g_atoms;
            (*ga).push(copy);
            if collect {
                atom_st().composite.queued = true;
            }
        }
        if collect {
            let comp_atoms = &raw mut atom_st().composite.atoms;
            (*comp_atoms).push(std::mem::take(atom));
        } else {
            if atom.r#type != kAInsertSpan {
                atom_emit(&*atom);
            }
            *atom = CmdAtom::new();
        }
    }
}

fn atom_push(cascade: bool, atom: &mut CmdAtom) {
     {
        if atom_blocked() {
            *atom = CmdAtom::new();
            return;
        }
        atom_push_raw(cascade && mc_buf_has_cursors(curbuf_mut()), &mut *atom);
    }
}

fn atom_stage_set(atom: &mut CmdAtom) {
    unsafe {
        let cur_frame = cur_frame_ptr();
        assert!(!cur_frame.is_null());
        assert!(!atom_staged());
        (*cur_frame).staged = CmdAtom::new();
        if atom_blocked() {
            *atom = CmdAtom::new();
            return;
        }
        assert!(atom.keys.is_some());
        (*cur_frame).staged = std::mem::take(&mut *atom);
    }
}

#[inline]
fn atom_staged() -> bool {
    unsafe {
        let cur_frame = cur_frame_ptr();
        !cur_frame.is_null() && (*cur_frame).staged.keys.is_some()
    }
}

fn atom_stage_flush(frame: &mut CmdFrame) {
    let Some(ref keys) = frame.staged.keys else {
            return;
        };
        let cascade = !keys.as_bytes().is_empty()
            && !frame.staged.cascaded
            && atom_st().global_ops == frame.global_ops;
        atom_push(cascade, &mut frame.staged);
        frame.staged = CmdAtom::default();
}

fn atom_composite_lhs() -> std::ffi::CString {
    unsafe {
        let mut keys: Vec<c_char> = Vec::new();
        sb_concat(&mut keys, ptr::addr_of!(atom_st().composite.lhs).cast());
        let typed_keys = &raw const atom_st().typed.keys;
        let tail = &(*typed_keys).as_slice()[atom_st().typed.map_start..];
        keys.extend_from_slice(std::slice::from_raw_parts(
            tail.as_ptr().cast::<c_char>(),
            tail.len(),
        ));
        let slice = crate::memory::cstr_slice(keys.as_ptr(), keys.len());
        std::ffi::CString::new(slice).unwrap()
    }
}

pub fn atom_lhs_replay_queue() {
    unsafe {
        let ga = &raw mut atom_st().g_atoms;
        let mut atom = CmdAtom::new();
        atom.r#type = kAComp;
        atom.keys = Some(atom_composite_lhs());
        atom.remap = true;
        atom.origin = atom_st().composite.origin;
        (*ga).push(atom);
    }
}

pub fn atom_composite_queued() -> bool {
    atom_st().composite.queued
}

pub fn atom_composite_active() -> bool {
    atom_st().composite.active
}

fn atom_composite_label(lhs: *const c_char, len: usize) {
    unsafe {
        let copy_len = len.min(MAXMAPLEN + 3);
        ptr::copy_nonoverlapping(lhs, ptr::addr_of_mut!(atom_st().composite.lhs).cast(), copy_len);
        atom_st().composite.lhs[copy_len] = 0;
    }
}

fn atom_composite_start(lhs: *const c_char, len: usize) {
    if !lhs.is_null() {
            atom_composite_label(lhs, len);
        }
        atom_st().composite.active = true;
        atom_st().composite.queued = false;
        atom_st().composite.lossy = false;
        atom_st().composite.frame = 0;
}

fn atom_composite_end() {
    unsafe {
        atom_st().composite.r#macro = false;
        if !atom_st().composite.active {
            return;
        }
        let comp_atoms = &raw mut atom_st().composite.atoms;
        let remap = atom_st().composite.lossy || (*comp_atoms).is_empty();
        let lhs = atom_composite_lhs();
        atom_st().composite.lhs[0] = 0;
        atom_st().composite.active = false;
        let mut atom: CmdAtom;
        if (*comp_atoms).len() == 1 {
            atom = (*comp_atoms).pop().unwrap();
            atom.lhs = Some(lhs);
            atom.remap = remap;
        } else {
            let keys_cstr = atoms_concat_keys(&*comp_atoms, 0);
            atom = CmdAtom::new();
            atom.r#type = kAComp;
            atom.keys = Some(keys_cstr);
            atom.lhs = Some(lhs);
            atom.remap = remap;
            atom.origin = atom_st().composite.origin;
            atom.changed = atom_origin_changed(atom_st().composite.origin);
            atom.moved = atom_origin_moved(atom_st().composite.origin);
            atom.undoseq = atom_origin_undoseq(atom_st().composite.origin);
            atom.atoms = std::mem::take(&mut *comp_atoms);
        }
        atom_emit(&atom);
    }
}

pub fn atom_composite_abort() {
    atom_st().composite.r#macro = false;
        atom_st().composite.lhs[0] = 0;
        atom_st().composite.active = false;
        atom_st().composite.atoms = Vec::new();
        atom_st().composite.open = false;
}

pub fn atom_term_enter() {
    if !mc_replaying() {
            atom_composite_end();
            atom_st().composite.open = false;
        }
}

pub fn atom_is_user_cmd() -> bool {
    !mc_replaying()
            && ((g_st().reg_executing == 0 && !g_st().pending_end_reg_executing) || atom_st().composite.r#macro)
            && g_st().ex_normal_busy == 0
}

#[inline]
fn atom_is_user_input() -> bool {
    g_st().KeyTyped || (atom_is_user_cmd() && atom_composite_active())
}

pub fn atom_suppress(suppress: bool) {
    atom_st().atom_suppressed = suppress;
}

#[inline]
fn atom_blocked() -> bool {
    mc_replaying() || atom_st().atom_suppressed
}

#[inline]
fn atom_capturable(consumers: bool, keytyped: bool) -> bool {
    consumers && atom_is_user_cmd() && (keytyped || atom_composite_active())
}

#[inline]
fn atom_buf_has_consumers() -> bool {
     mc_buf_has_cursors(curbuf_mut()) || has_event(EVENT_CMDATOM)
}

pub fn atom_key_class(cmd: c_int, arg: c_int, arg2: c_int) -> c_uint {
    if cmd == K_EVENT || cmd == K_IGNORE {
            return kKeyOpaque | kKeySynthetic;
        }
        if cmd == K_COMMAND || cmd == K_LUA {
            return kKeyOpaque;
        }
        if cmd == b'/' as c_int
            || cmd == b'?' as c_int
            || cmd == b':' as c_int
            || cmd == b'!' as c_int
        {
            return kKeyPayload;
        }
        if cmd == Ctrl_D || cmd == Ctrl_U {
            return kKeyScrollMove | kKeyInsFlush;
        }
        if cmd == Ctrl_F || cmd == Ctrl_B {
            return kKeyScrollMove;
        }
        if cmd == Ctrl_E || cmd == Ctrl_Y {
            return kKeyScrollView;
        }
        if cmd == Ctrl_T {
            return kKeyInsFlush;
        }
        if cmd == b'g' as c_int {
            if arg == b'\'' as c_int || arg == b'`' as c_int {
                return atom_key_class(arg, arg2, NUL);
            }
            if b";,go".contains(&(arg as u8)) {
                return kKeyJump;
            }
            return if b"jk0^$_meEM*#".contains(&(arg as u8)) {
                kKeyMotion
            } else {
                0
            };
        }
        if cmd == b'[' as c_int || cmd == b']' as c_int {
            if arg == b'C' as c_int {
                return kKeyJump;
            }
            return if b"[](){}mMcsz#*/".contains(&(arg as u8)) {
                kKeyMotion
            } else {
                0
            };
        }
        if cmd == b'z' as c_int {
            return if arg == b'j' as c_int || arg == b'k' as c_int {
                kKeyMotion
            } else {
                0
            };
        }
        if cmd == b'\'' as c_int || cmd == b'`' as c_int {
            return if arg != NUL && b"<>[].^(){}".contains(&(arg as u8)) {
                kKeyMotion
            } else {
                kKeyJump
            };
        }
        if cmd == K_DOWN
            || cmd == K_END
            || cmd == K_HOME
            || cmd == K_LEFT
            || cmd == K_RIGHT
            || cmd == K_UP
        {
            return kKeyMotion | kKeyInsFlush;
        }
        if cmd == Ctrl_G
            || cmd == Ctrl_H
            || cmd == Ctrl_R
            || cmd == Ctrl_W
            || cmd == K_BS
            || cmd == K_DEL
            || cmd == K_S_LEFT
            || cmd == K_S_RIGHT
        {
            return kKeyInsFlush;
        }
        if cmd == K_LEFTMOUSE
            || cmd == K_LEFTMOUSE_NM
            || cmd == K_MIDDLEMOUSE
            || cmd == K_RIGHTMOUSE
            || cmd == K_X1MOUSE
            || cmd == K_X2MOUSE
        {
            return kKeyMouse;
        }
        if cmd == K_MOUSEDOWN || cmd == K_MOUSEUP || cmd == K_MOUSELEFT || cmd == K_MOUSERIGHT {
            return kKeyScrollView;
        }
        0
}

pub fn atom_cmdline_set(firstc: c_int, line: *const c_char, len: usize) {
    unsafe {
        let cur_frame = cur_frame_ptr();
        if cur_frame.is_null()
            || !atom_is_user_cmd()
            || (firstc != b':' as c_int && firstc != K_COMMAND)
        {
            return;
        }
        let root = root_frame();
        if (*root).cmdline.is_none() {
            let slice = crate::memory::cstr_slice(line, len);
            (*root).cmdline = Some(std::ffi::CString::new(slice).unwrap());
        }
    }
}

#[inline]
fn atom_typed_collecting() -> bool {
    unsafe {
        let cur_frame = cur_frame_ptr();
        atom_composite_active()
            || (!cur_frame.is_null() && (*cur_frame).payload_start != usize::MAX)
    }
}

pub fn atom_payload_start() {
    unsafe {
        let cur_frame = cur_frame_ptr();
        if g_st().expr_map_lock == 0
            && !cur_frame.is_null()
            && (*cur_frame).payload_start == usize::MAX
            && !mc_replaying()
        {
            let typed_keys = &raw const atom_st().typed.keys;
            let len = (*typed_keys).len();
            (*cur_frame).payload_start = len;
            (*cur_frame).payload_end = len;
        }
    }
}

pub fn atom_payload_end() {
    unsafe {
        let cur_frame = cur_frame_ptr();
        if g_st().expr_map_lock == 0
            && !cur_frame.is_null()
            && (*cur_frame).payload_start != usize::MAX
            && !mc_replaying()
        {
            let typed_keys = &raw const atom_st().typed.keys;
            (*cur_frame).payload_end = (*typed_keys).len();
        }
    }
}

fn atom_payload_append(atom: &mut CmdAtom, frame: &mut CmdFrame) {
    unsafe {
        let plen = if frame.payload_start == usize::MAX {
            0
        } else {
            frame.payload_end - frame.payload_start
        };
        let Some(ref cur_keys) = atom.keys else {
            return;
        };
        if plen == 0 {
            return;
        }
        let mut bytes = cur_keys.as_bytes().to_vec();
        let typed_keys = &raw const atom_st().typed.keys;
        bytes.extend_from_slice(&(&*typed_keys)[frame.payload_start..frame.payload_end]);
        atom.keys = Some(std::ffi::CString::new(bytes).unwrap());
        frame.payload_start = usize::MAX;
    }
}

pub fn atom_typed_add(chars: *const u8, len: usize) {
    unsafe {
        if mc_replaying() || !atom_typed_collecting() || g_st().cmdline_star > 0 {
            return;
        }
        if len == 3
            && *chars == K_SPECIAL
            && (atom_key_class(
                termcap2key(c_int::from(*chars.add(1)), c_int::from(*chars.add(2))),
                NUL,
                NUL,
            ) & kKeySynthetic)
                != 0
        {
            return;
        }
        if len > 0 {
            let typed_keys = &raw mut atom_st().typed.keys;
            (*typed_keys).extend_from_slice(std::slice::from_raw_parts(chars, len));
        }
    }
}

pub fn atom_typed_del(len: usize) {
    unsafe {
        if mc_replaying() || !atom_typed_collecting() {
            return;
        }
        let typed_keys = &raw mut atom_st().typed.keys;
        let new_len = (*typed_keys).len().saturating_sub(len);
        (*typed_keys).truncate(new_len);
    }
}

pub fn atom_did_global_op() {
    if !mc_replaying() {
            atom_st().global_ops += 1;
            atom_st().composite.queued = true;
        }
}

pub fn atom_follow_changed() {
    unsafe {
        if mc_replaying() {
            return;
        }
        let frames = &raw mut atom_st().cmd_frames;
        for frame in (*frames).iter_mut().rev() {
            if !atom_origin_moved(frame.origin) {
                frame.follow = mc_following();
            } else if atom_st().composite.follow == kNone {
                atom_st().composite.follow = if frame.follow { kTrue } else { kFalse };
            }
        }
    }
}

pub fn atom_redo_prepped() {
    unsafe {
        if atom_is_user_cmd() {
            let cur_id = (*cur_frame_ptr()).id;
            (*root_frame()).redo_frame = cur_id;
        }
    }
}

pub fn atom_redo_cancel() {
    unsafe {
        (*root_frame()).redo_frame = 0;
    }
}

pub fn atom_macro_start(regname: c_int) {
    if atom_is_user_input() {
            atom_st().composite.r#macro = true;
            if !atom_composite_active() {
                let lhs: [c_char; 3] = [b'@' as c_char, regname as c_char, 0];
                atom_composite_start(lhs.as_ptr(), 2);
            }
        }
}

pub fn atom_stuff_start(cap: &cmdarg_T) {
    if mc_replaying() || atom_composite_active() || !atom_is_user_input() {
            return;
        }
        let lhs = atom_redo_keys(&atom_cmd_spec(cap));
        atom_composite_start(lhs.as_ptr(), lhs.as_bytes().len());
}

pub fn atom_map_start(lhs: *const c_char, len: usize, peeked: bool) {
    unsafe {
        if mc_replaying() || g_st().reg_executing != 0 || g_st().ex_normal_busy != 0 {
            return;
        }
        if (g_st().State & MODE_NORMAL) == 0 {
            atom_composite_label(lhs, len);
            return;
        }
        let typed_keys = &raw const atom_st().typed.keys;
        if atom_composite_active() {
            if get_real_state() == MODE_OP_PENDING {
                return;
            }
            atom_st().typed.map_start = (*typed_keys).len();
            atom_composite_end();
        }
        let op = get_vim_var_str(VV_OP);
        if get_real_state() == MODE_OP_PENDING && *op != 0 {
            assert!(*lhs.add(len) == 0);
            let mut full_bytes = Vec::from(crate::memory::cstr_as_bytes(op));
            full_bytes.extend_from_slice(crate::memory::cstr_slice(lhs, len));
            let full = cstr_from_bytes(&full_bytes);
            atom_composite_start(full.as_ptr(), full.as_bytes().len());
            atom_st().typed.map_start = (*typed_keys).len();
            return;
        }
        atom_composite_start(lhs, len);
        let cur_frame = cur_frame_ptr();
        if peeked && !cur_frame.is_null() {
            atom_st().composite.frame = (*cur_frame).id;
        }
        atom_st().typed.map_start = (*typed_keys).len();
    }
}

pub fn atom_visual_reset() {
    atom_st().vatom.state = kVatomNone;
        atom_st().vatom.atoms = Vec::new();
        atom_st().vatom.origin = CmdOrigin::default();
        atom_st().vatom.frame = 0;
        atom_st().vatom.done = 0;
        mc_vsel_clear();
}

#[inline]
fn atom_visual_pending() -> bool {
    atom_st().vatom.state != kVatomNone
}

#[inline]
fn atom_visual_typed() -> bool {
    atom_visual_pending()
            && ((atom_st().vatom.state & kVatomTyped) != 0 || g_st().maptick != atom_st().vatom.origin.maptick)
}

pub fn atom_visual_replayable() -> bool {
    atom_visual_pending() && (atom_st().vatom.state & kVatomVoid) == 0
}

pub fn atom_visual_redoable() -> bool {
    unsafe {
        if !atom_visual_replayable() {
            return false;
        }
        let v_atoms = &raw const atom_st().vatom.atoms;
        for atom in &*v_atoms {
            if atom.spec.cmd == b'g' as c_int && atom.spec.cmd2 == b'v' as c_int {
                return false;
            }
        }
        true
    }
}

pub fn atom_visual_span() -> Option<std::ffi::CString> {
    unsafe {
        if !atom_visual_replayable() {
            return None;
        }
        let v_atoms = &raw const atom_st().vatom.atoms;
        let tail = atoms_concat_keys(&*v_atoms, atom_st().vatom.done);
        if atom_st().vatom.done == 0 {
            return Some(tail);
        }

        let mut keys: Vec<u8> = Vec::with_capacity(2 + tail.as_bytes().len());
        keys.extend_from_slice(b"gv");
        keys.extend_from_slice(tail.as_bytes());
        Some(cstr_from_bytes(&keys))
    }
}

fn atom_visual_span_flush() {
    unsafe {
        let v_atoms = &raw const atom_st().vatom.atoms;
        if atom_st().vatom.done >= (*v_atoms).len()
            || !atom_visual_replayable()
            || !atom_visual_typed()
            || !mc_buf_has_cursors(curbuf_mut())
        {
            return;
        }
        let mut edited = false;
        for atom in &(*v_atoms).as_slice()[atom_st().vatom.done..] {
            assert!(atom.r#type != kAOperator);
            edited |= atom.changed;
        }
        if !edited {
            return;
        }
        let span_cstr = atom_visual_span();
        atom_st().vatom.done = (*v_atoms).len();
        let ga = &raw mut atom_st().g_atoms;
        let mut atom = CmdAtom::new();
        atom.r#type = kAVisualSpan;
        atom.keys = span_cstr;
        atom.origin = atom_st().vatom.origin;
        (*ga).push(atom);
    }
}

fn atom_visual_end_suffix(
    suffix: Option<std::ffi::CString>,spec: Option<&CmdSpec>,
    redoable: bool,
) -> bool {
    unsafe {
        if mc_replaying() || atom_st().atom_suppressed {
            return false;
        }
        let origin = atom_st().vatom.origin;
        let v_atoms = &raw mut atom_st().vatom.atoms;
        let replayable = suffix.is_some() && atom_visual_replayable();
        let v = if replayable {
            Some(atoms_concat_keys(&*v_atoms, 0))
        } else {
            None
        };
        let prepped =
            redoable && spec.is_some() && suffix.is_some() && (replayable || spec.unwrap().op != NUL);

        if prepped {
            let use_redo_keys = atom_visual_redoable();
            let (redo_ptr, redo_len) = if use_redo_keys && let Some(ref vc) = v {
                (vc.as_ptr(), vc.as_bytes().len())
            } else {
                (c"1v".as_ptr(), 2)
            };
            prep_redo_visual(redo_ptr, redo_len, CmdSpec::default());
            if let Some(ref suf) = suffix {
                redo_append_str(suf.as_ptr(), -1);
            }
        }

        if is_child_frame() {
            return prepped;
        }

        if !replayable {
            let emit = (atom_st().vatom.state & kVatomVoid) != 0
                && atom_visual_typed()
                && spec.is_some()
                && suffix.is_some()
                && atom_is_user_cmd();
            let mut label: Option<std::ffi::CString> = None;
            if emit {
                let collected = atoms_concat_keys(&*v_atoms, 0);
                let mut bytes = Vec::from(collected.as_bytes());
                if let Some(ref suf) = suffix {
                    bytes.extend_from_slice(suf.as_bytes());
                }
                label = Some(cstr_from_bytes(&bytes));
            }
            atom_visual_reset();
            if emit {
                let mut staged_atom = CmdAtom::new();
                staged_atom.r#type = kAVisual;
                staged_atom.spec = spec.unwrap().without_body();
                staged_atom.keys = Some(std::ffi::CString::default());
                staged_atom.lhs = label;
                staged_atom.origin = origin;
                atom_stage_set(&mut staged_atom);
            }
            return prepped;
        }

        if !atom_is_user_cmd() || !atom_visual_typed() {
            atom_visual_reset();
            return prepped;
        }

        let mut keys_bytes = Vec::from(v.as_ref().map_or(&[][..], |vc| vc.as_bytes()));
        if let Some(ref suf) = suffix {
            keys_bytes.extend_from_slice(suf.as_bytes());
        }
        let keys_cstr = cstr_from_bytes(&keys_bytes);
        let mut atom = CmdAtom::new();
        atom.r#type = kAVisual;
        if let Some(spec) = spec {
            atom.spec = spec.without_body();
        }
        atom.keys = Some(keys_cstr);
        atom.origin = origin;
        if let Some(spec) = spec {
            let mut op_atom = CmdAtom::new();
            op_atom.r#type = kAOperator;
            op_atom.spec = spec.without_body();
            op_atom.keys = suffix;
            op_atom.origin = (*cur_frame_ptr()).origin;
            (*v_atoms).push(op_atom);
        }
        atom.cascaded = atom_st().vatom.done > 0;
        if atom.cascaded && atom_st().vatom.done < (*v_atoms).len() {
            let span_cstr = atom_visual_span();
            let ga = &raw mut atom_st().g_atoms;
            let mut span_atom = CmdAtom::new();
            span_atom.r#type = kAVisualSpan;
            span_atom.keys = span_cstr;
            span_atom.origin = origin;
            (*ga).push(span_atom);
        }
        atom.atoms = std::mem::take(&mut *v_atoms);
        atom_visual_reset();
        atom_stage_set(&mut atom);
        prepped
    }
}

pub fn atom_visual_end(spec: CmdSpec, redoable: bool) -> bool {
    if mc_replaying() {
            return false;
        }
        let suffix = atom_redo_keys(&spec);
        atom_visual_end_suffix(Some(suffix), Some(&spec), redoable)
}

pub fn atom_capture_op(oap: &mut oparg_T, cap: &mut cmdarg_T, redo_yank: bool) {
    unsafe {
        let redoable = op_redoable(oap.op_type, redo_yank);
        let ins_op = oap.op_type == OP_CHANGE
            || oap.op_type == OP_INSERT
            || oap.op_type == OP_APPEND;
        let payload_motion = cap.cmdchar >= 0x100
            || (cap.cmdchar != NUL && b"/?:!".contains(&(cap.cmdchar as u8)));
        let excmd = cap.cmdchar == b':' as c_int || cap.cmdchar == K_COMMAND;
        if !ins_op && !payload_motion {
            let prep_exempt = !redoable || cap.cmdchar == b'D' as c_int;
            let mut spec = CmdSpec {
                regname: oap.regname,
                count: c_long::from(cap.count0),
                op: get_op_char(oap.op_type),
                op_extra: get_extra_op_char(oap.op_type),
                ..CmdSpec::default()
            };
            if prep_exempt && (!g_st().Visual.active || oap.motion_force != 0) {
                if atom_capturable(atom_buf_has_consumers(), g_st().KeyTyped) {
                    let operand = nv_is(cap.cmdchar, NV_LANG | NV_NCH_ARG);
                    spec.motion_force = oap.motion_force;
                    spec.cmd = cap.cmdchar;
                    spec.cmd2 = if operand { NUL } else { cap.nchar };
                    spec.cmdarg = if operand { cap.nchar } else { NUL };
                    let mut op_atom = atom_from_spec(kAOperator, spec);
                    op_atom.origin = (*cur_frame_ptr()).origin;
                    atom_stage_set(&mut op_atom);
                }
            } else if !g_st().Visual.active || oap.motion_force != 0 {
                // Prepped: atom_cmd_end() derives the atom from redobuff.
            } else if oap.op_type == OP_REPLACE && cap.nchar <= 0 {
                let suffix: [c_char; 4] = [
                    b'r' as c_char,
                    Ctrl_V as c_char,
                    if cap.nchar == REPLACE_CR_NCHAR {
                        CAR as c_char
                    } else {
                        NL as c_char
                    },
                    0,
                ];
                atom_visual_end_suffix(Some(cstr_to_cstring(suffix.as_ptr())), Some(&spec), redoable);
            } else {
                spec.cmdarg = if oap.op_type == OP_REPLACE {
                    cap.nchar
                } else {
                    NUL
                };
                if (oap.op_type == OP_NR_ADD || oap.op_type == OP_NR_SUB) && cap.arg != 0 {
                    spec.op_extra = spec.op;
                    spec.op = b'g' as c_int;
                }
                atom_visual_end(spec, redoable);
            }
        }

        if redoable
            && cap.cmdchar != b'D' as c_int
            && ((!g_st().Visual.active || oap.motion_force != 0) || excmd || cap.cmdchar == K_LUA)
        {
            prep_redo(
                true,
                false,
                CmdSpec {
                    regname: oap.regname,
                    count: c_long::from(cap.count0),
                    op: get_op_char(oap.op_type),
                    op_extra: get_extra_op_char(oap.op_type),
                    motion_force: oap.motion_force,
                    cmd: cap.cmdchar,
                    cmd2: cap.nchar,
                    ..CmdSpec::default()
                },
            );
            if cap.cmdchar == b'/' as c_int || cap.cmdchar == b'?' as c_int {
                if vim_strchr(opt_vars().p_cpo, kCpoRedo).is_null() {
                    redo_append_lit(cap.searchbuf_ptr(), -1);
                }
                redo_append_str(c"\n", 1);
            } else if excmd {
                let rc = g_st().repeat_cmdline.take();
                if let Some(ref rc) = rc {
                    if cap.cmdchar == b':' as c_int {
                        redo_append_lit(rc.as_ptr(), -1);
                    } else {
                        redo_append_spec(rc.as_ptr());
                    }
                    redo_append_str(c"\n", 1);
                } else {
                    redo_new(CmdSpec::default());
                }
            } else if cap.cmdchar == K_LUA {
                let mut lua_id = [0 as c_char; NUMBUFLEN];
                let n = vim_snprintf!(
                    lua_id.as_mut_ptr(),
                    lua_id.len(),
                    c"%d",
                    g_st().repeat_luamap,
                );
                redo_append_str(lua_id.as_ptr(), n as isize);
                redo_append_str(c"\n", 1);
            }
        } else if g_st().Visual.active && redoable && oap.motion_force == NUL {
            if op_self_select(cap) {
                prep_redo(
                    true,
                    false,
                    CmdSpec {
                        regname: oap.regname,
                        count: c_long::from(cap.count0),
                        op: get_op_char(oap.op_type),
                        op_extra: get_extra_op_char(oap.op_type),
                        motion_force: oap.motion_force,
                        cmd: cap.cmdchar,
                        cmd2: cap.nchar,
                        ..CmdSpec::default()
                    },
                );
            } else if ins_op && !excmd && cap.cmdchar != K_LUA {
                let v = if atom_visual_redoable() {
                    atom_visual_span()
                } else {
                    None
                };
                let (v_ptr, v_len) = if let Some(ref vc) = v {
                    (vc.as_ptr(), vc.as_bytes().len())
                } else {
                    (c"1v".as_ptr(), 2)
                };
                prep_redo_visual(
                    v_ptr,
                    v_len,
                    CmdSpec {
                        regname: oap.regname,
                        op: get_op_char(oap.op_type),
                        op_extra: get_extra_op_char(oap.op_type),
                        ..CmdSpec::default()
                    },
                );
            }
        }
    }
}

pub fn atom_ins_start(
    cmd: c_int,
    count: c_long,
    vis: VisualIns,
    vblock: bool,
) -> InsSession {
    unsafe {
        let cur_frame = cur_frame_ptr();
        let mut session = InsSession {
            typed: atom_is_user_input(),
            vis,
            origin: if vis == kVInsKeys {
                atom_st().vatom.origin
            } else {
                (*cur_frame).origin
            },
            vsel: None,
        };

        if vis != kVInsNone && !mc_replaying() && !is_child_frame() {
            if vis == kVInsOther && atom_visual_replayable() && !atom_visual_redoable() {
                session.vsel = atom_visual_span();
                session.origin = atom_st().vatom.origin;
            }
            if vis == kVInsKeys || session.vsel.is_some() {
                session.typed = atom_visual_replayable() && atom_visual_typed();
            }
            atom_visual_reset();
        }

        let repl = cmd == b'R' as c_int
            || cmd == b'V' as c_int
            || cmd == b'r' as c_int
            || cmd == b'v' as c_int;
        let reexec = vis == kVInsNone
            || vis == kVInsKeys
            || session.vsel.is_some()
            || (vis == kVInsMotion && (*cur_frame).payload_start == usize::MAX);
        mc_ins_cascade_start(
            session.typed && count <= 1 && !repl && !vblock && reexec,
            session.origin,
            (*root_frame()).id,
            session.vsel.clone(),
        );

        session
    }
}

pub fn atom_ins_end(session: &mut InsSession, busy: bool) {
     {
        let visual = session.vis != kVInsNone;
        let user_input = session.typed || g_st().maptick != session.origin.maptick;

        if mc_replaying() {
            session.vsel = None;
            return;
        }
        if mc_ins_commit() {
            if has_event(EVENT_CMDATOM) && !atom_composite_active() {
                atom_ins_push(session, false);
            }
            session.vsel = None;
            return;
        }
        if !user_input
            || busy
            || g_st().restart_edit != 0
            || !atom_buf_has_consumers()
            || (visual && session.vis != kVInsKeys && session.vsel.is_none())
        {
            if user_input && (busy || g_st().restart_edit != 0) && atom_composite_active() {
                atom_st().composite.lossy = true;
            }
            session.vsel = None;
            return;
        }
        atom_ins_push(session, mc_buf_has_cursors(curbuf_mut()));
        session.vsel = None;
    }
}

fn atom_ins_push(session: &InsSession, cascade: bool) {
    let mut atom = atom_from_redo(if session.vis != kVInsNone {
            kAVisual
        } else {
            kAInsert
        });
        if let Some(ref vsel) = session.vsel
            && let Some(ref cur_keys) = atom.keys
        {
            assert!(cur_keys.as_bytes().starts_with(b"1v"));
            let mut bytes = Vec::from(vsel.as_bytes());
            bytes.extend_from_slice(&cur_keys.as_bytes()[2..]);
            atom.keys = Some(cstr_from_bytes(&bytes));
        }
        let Some(ref cur_keys) = atom.keys else {
            return;
        };
        let key_bytes = cur_keys.as_bytes();
        let replace = atom.spec.cmd == b'r' as c_int
            || (atom.spec.cmd == b'g' as c_int && atom.spec.cmd2 == b'r' as c_int);
        if key_bytes.is_empty() || (!replace && key_bytes[key_bytes.len() - 1] != ESC) {
            return;
        }
        atom.text = get_last_insert_save();
        atom.origin = session.origin;
        atom_push_raw(cascade, &mut atom);
}

pub fn atom_cmd_start(cmdchar: c_int) {
    unsafe {
        let consumers = atom_buf_has_consumers();
        atom_st().frame_id += 1;
        let frames = &raw mut atom_st().cmd_frames;
        let is_root = (*frames).is_empty();
        let origin = atom_origin();
        (*frames).push(CmdFrame {
            origin,
            staged: CmdAtom::default(),
            visual: g_st().Visual,
            keytyped: g_st().KeyTyped,
            keyclass: atom_key_class(cmdchar, NUL, NUL),
            ex_normal: g_st().ex_normal_busy,
            captures: atom_st().atom_captures,
            global_ops: atom_st().global_ops,
            beeps: g_st().did_beep,
            id: atom_st().frame_id,
            follow: mc_following(),
            consumers,
            reg_ts: if consumers { reg_max_ts(true) } else { 0 },
            payload_start: usize::MAX,
            payload_end: 0,
            redo_frame: 0,
            cmdline: None,
        });
        if is_root && !atom_st().composite.open {
            atom_st().composite.open = true;
            atom_st().composite.origin = origin;
            atom_st().composite.follow = kNone;
        }
        if is_root && !mc_replaying() && !atom_st().composite.active && atom_st().composite.lhs[0] != 0 {
            if !g_st().KeyTyped && consumers {
                atom_composite_start(ptr::null(), 0);
                let typed_keys = &raw const atom_st().typed.keys;
                atom_st().typed.map_start = (*typed_keys).len();
                atom_st().composite.follow = kNone;
            } else {
                atom_st().composite.lhs[0] = 0;
            }
        }
        if is_root && atom_is_user_cmd() && !atom_composite_active() {
            let typed_keys = &raw mut atom_st().typed.keys;
            (*typed_keys).clear();
            atom_st().typed.map_start = 0;
        }
    }
}

fn atom_capture_cmd(ca: &mut cmdarg_T, old: &mut CmdFrame, is_root: bool) -> bool {
    unsafe {
        if mc_replaying() || atom_st().atom_suppressed {
            return false;
        }

        let user = atom_is_user_cmd();
        let root = root_frame();
        let keycls = atom_key_class(ca.cmdchar, ca.nchar, ca.extra_char);
        let opaque = (keycls & kKeyOpaque) != 0;
        let synthetic = (keycls & kKeySynthetic) != 0;
        let unchanged = g_st().curbuf == old.origin.buf.br_buf
            && ptr::eq((curwin_ptr()).cast_const(), old.origin.win)
            && !atom_origin_moved(old.origin)
            && !atom_origin_changed(old.origin)
            && g_st().Visual.active == old.visual.active
            && (!g_st().Visual.active
                || (equalpos(old.visual.start, g_st().Visual.start)
                    && old.visual.mode == g_st().Visual.mode));
        if opaque
            && unchanged
            && !(atom_visual_pending() && atom_st().frame_id > old.id)
            && (*root).redo_frame != old.id
        {
            return false;
        }
        let ins_cascaded = user && mc_ins_cascaded((*root).id);
        let mapped = user && !old.keytyped && !synthetic;
        if mapped
            && atom_st().global_ops == old.global_ops
            && ((g_st().curbuf == old.origin.buf.br_buf && atom_origin_changed(old.origin))
                || ins_cascaded)
        {
            atom_st().map_edit = true;
        }

        let child = is_child_frame();
        let mut vis = false;
        if g_st().Visual.active && !child {
            if !old.visual.active && atom_st().vatom.frame <= old.id {
                atom_visual_reset();
                atom_st().vatom.state = kVatomFed;
                atom_st().vatom.origin = old.origin;
            }
            if atom_visual_pending()
                && user
                && !synthetic
                && (old.keytyped || atom_composite_active())
            {
                atom_st().vatom.state = (atom_st().vatom.state & kVatomVoid) | kVatomTyped;
            }
            vis = atom_visual_pending()
                && ca.cmdchar != b'Q' as c_int
                && ca.cmdchar != b'q' as c_int;
            if vis
                && (g_st().Visual.select
                    || ((keycls & (kKeyScrollMove | kKeyScrollView | kKeyMouse)) != 0
                        && !unchanged))
            {
                atom_st().vatom.state |= kVatomVoid;
            }
        } else if old.visual.active && !child {
            let mut own = if atom_visual_pending() {
                atom_from_frame(ca, old)
            } else {
                CmdAtom::default()
            };
            let v_atoms = &raw const atom_st().vatom.atoms;
            if let Some(suffix) = own.keys.take() {
                drop(own);
                atom_visual_end_suffix(Some(suffix), None, false);
            } else if user && atom_visual_replayable() && !(*v_atoms).is_empty() {
                let suffix: [c_char; 2] = [ESC as c_char, 0];
                atom_visual_end_suffix(
                    Some(cstr_to_cstring(suffix.as_ptr())),
                    None,
                    false,
                );
            } else {
                atom_visual_end_suffix(None, None, false);
            }
        }

        let v_atoms = &raw const atom_st().vatom.atoms;
        if (vis
            && atom_st().vatom.frame <= old.id
            && atom_st().atom_captures == old.captures
            && (*ca.oap).op_type == OP_NOP)
            || (!g_st().Visual.active
                && !old.visual.active
                && atom_capturable(old.consumers, old.keytyped)
                && atom_st().atom_captures == old.captures
                && !atom_staged()
                && (*ca.oap).op_type == OP_NOP
                && stuff_empty()
                && !ins_cascaded)
        {
            let collected = (*v_atoms).len();
            let special_motion = (keycls & kKeyMotion) != 0;
            let scroll_cmd = (keycls & (kKeyScrollMove | kKeyScrollView)) != 0;
            let mouse_cmd = (keycls & kKeyMouse) != 0;
            let jump_cmd = nv_is(ca.cmdchar, NV_JUMP)
                || (keycls & kKeyJump) != 0
                || (ca.cmdchar == b'%' as c_int && ca.count0 > 0);
            let replayable = (ca.cmdchar > 0
                && ca.cmdchar < 0x100
                && ca.cmdchar != b'"' as c_int
                && ca.cmdchar != b'@' as c_int
                && !scroll_cmd)
                || special_motion;
            let changed = atom_origin_changed(old.origin);
            let motion = (nv_is(ca.cmdchar, NV_MOTION) || special_motion)
                && !changed
                && !g_st().finish_op
                && !jump_cmd;
            let failed = g_st().did_beep != old.beeps && !atom_origin_moved(old.origin);
            let follow = (mc_following() || mapped) && motion && !failed;

            if (*root).redo_frame == old.id && !mouse_cmd {
                let mut atom = atom_from_redo(kAOperator);
                atom_payload_append(&mut atom,  old);

                let effect = (changed
                    || reg_max_ts(true) > old.reg_ts
                    || (mc_following() && atom_origin_moved(old.origin)))
                    && atom_st().global_ops == old.global_ops;
                if atom.keys.as_ref().is_some_and(|k| !k.as_bytes().is_empty()) {
                    atom.origin = old.origin;
                    atom_push(effect, &mut atom);
                }
            } else if ca.searchbuf.is_some()
                && (ca.cmdchar == b'/' as c_int || ca.cmdchar == b'?' as c_int)
                && !(vis && unchanged)
            {
                let mut atom =
                    atom_from_cmdline(kAMotion, ca, ca.searchbuf_ptr(), old.visual.active);
                atom.origin = old.origin;
                atom_push(false, &mut atom);
            } else if ((*root).cmdline.is_some()
                && (ca.cmdchar == b':' as c_int || ca.cmdchar == K_COMMAND))
                || (vis && ca.cmdchar == K_LUA)
            {
                let mut atom = atom_from_frame(ca, old);
                atom.origin = old.origin;
                atom_push(false, &mut atom);
            } else if replayable && (!vis || ((keycls & kKeyPayload) == 0 && !failed)) {
                let mut spec = atom_cmd_spec(ca);
                if vis {
                    spec.regname = 0;
                }
                let mut atom = atom_from_spec(
                    if motion {
                        kAMotion
                    } else if jump_cmd {
                        kAJump
                    } else {
                        kANormal
                    },
                    spec,
                );
                atom.origin = old.origin;
                atom_push(follow, &mut atom);
                if jump_cmd && atom_composite_active() {
                    atom_st().composite.queued = true;
                }
            } else if (scroll_cmd || mouse_cmd) && !atom_composite_active() {
                let mut spec = atom_cmd_spec(ca);
                if is_special(ca.cmdchar) {
                    spec.count = 0;
                }
                let mut atom = atom_from_spec(if scroll_cmd { kAScroll } else { kAMouse }, spec);
                atom.origin = old.origin;
                atom_push(false, &mut atom);
            }
            if vis && (*v_atoms).len() == collected && !unchanged {
                atom_st().vatom.state |= kVatomVoid;
            }
        }

        if vis
            && (g_st().curbuf != old.origin.buf.br_buf
                || (atom_origin_changed(old.origin) && atom_st().vatom.frame != old.id))
        {
            atom_st().vatom.state |= kVatomVoid;
        }
        atom_visual_span_flush();

        g_st().Visual.active && user && is_root
    }
}

pub fn atom_cmd_end(ca: &mut cmdarg_T) {
    unsafe {
        let frames = &raw mut atom_st().cmd_frames;
        let is_root = (*frames).len() == 1;
        let old = cur_frame_ptr();
        if atom_st().composite.follow == kNone
            && !mc_replaying()
            && (atom_origin_moved((*old).origin) || nv_is(ca.cmdchar, NV_MOTION))
        {
            atom_st().composite.follow = if (*old).follow { kTrue } else { kFalse };
        }
        let refresh_visual = atom_capture_cmd(ca,  &mut *old, is_root);
        atom_stage_flush(&mut *old);
        if atom_composite_active()
            && (*old).payload_start != usize::MAX
            && (*old).payload_end > (*old).payload_start
        {
            atom_st().composite.lossy = true;
        }

        if is_root && !mc_replaying() && typebuf_typed() != 0 && stuff_empty() {
            let map_moved = atom_composite_active() && atom_origin_moved(atom_st().composite.origin);
            mc_clock_edge(atom_st().map_edit, map_moved, atom_st().composite.follow == kTrue);

            atom_st().map_edit = false;
            if (*ca.oap).op_type == OP_NOP && !g_st().Visual.active && g_st().restart_edit == 0 {
                atom_composite_end();
                atom_st().composite.open = false;
            }
        }
        if refresh_visual {
            mc_vsel_refresh();
        }
        (*frames).pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sb_concat_works() {
         {
            let mut sb: Vec<c_char> = Vec::new();
            sb_concat(&mut sb, c"abc");
            sb_concat(&mut sb, c"def");
            assert_eq!(sb.len(), 6);
            assert_eq!(
                crate::memory::cstr_slice(sb.as_ptr(), 6),
                b"abcdef"
            );

            let mut v = CmdAtomVec::new();
            v.clear();
            assert!(v.is_empty());
        }
    }
}
