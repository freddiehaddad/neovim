//! Input engine command-atom policy layer (`src/nvim/input_cmdatom.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{
    api_free_dict, array_obj, boolean_obj, cstr_to_obj, cstr_to_string, dict_obj, integer_obj,
};
use crate::api::private::validate::{Array, Dict, KeyValuePair, Object};
use crate::autocmd::{AUGROUP_ALL, EVENT_CMDATOM, aucmd_defer, has_event};
use crate::buffer::{Bufref, FileBuf, buf_get_changedtick, bufref_valid, set_bufref};
use crate::charset::is_special;
use crate::cursor::{CursorPos, equalpos};
use crate::eval::typval::Varnumber;
use crate::eval::vars::{VV_OP, get_vim_var_str};
use crate::extmark::{K_FALSE, K_NONE, K_TRUE, TriState};
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
use crate::memory::bytes_to_cstring;
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
use crate::strings::format_to_buf;
use crate::textformat::Oparg;
use crate::viml::parser::expressions::ESC;
use crate::window::win_valid;
use std::ffi::CStr;

pub type CmdAtomVec = Vec<CmdAtom>;
use crate::os::time::Timestamp;

use crate::api::ui::MAX_SCHAR_SIZE;
use crate::cmdexpand::MB_MAXBYTES;
use crate::ex_cmds::{CTRL_E, CTRL_Y};
use crate::ex_getln::{
    CTRL_B_CONST, CTRL_D_CONST, CTRL_F_CONST, CTRL_G, CTRL_R_CONST, CTRL_T_CONST, CTRL_U,
    CTRL_W_CONST, K_EVENT, K_LUA,
};
use crate::mapping::MAXMAPLEN;
use crate::regexp::{CTRL_H_CONST, CTRL_V_CONST, NUMBUFLEN};

#[derive(Clone)]
pub struct Cmdarg {
    pub oap: crate::globals::GraphPtr<Oparg>,
    pub cmdchar: i32,
    pub nchar: i32,
    pub nchar_composing: [u8; MAX_SCHAR_SIZE],
    pub nchar_len: i32,
    pub extra_char: i32,
    pub opcount: i32,
    pub count0: i32,
    pub count1: i32,
    pub arg: i32,
    pub retval: i32,
    pub searchbuf: Option<std::ffi::CString>,
}

impl Cmdarg {
    pub const fn new() -> Self {
        Self {
            oap: crate::globals::GraphPtr::NULL,
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
    pub fn searchbuf_cstr(&self) -> Option<&CStr> {
        self.searchbuf.as_deref()
    }

    #[inline]
    pub fn oap_mut(&self) -> &'static mut Oparg {
        self.oap.borrow_mut()
    }
}

impl Default for Cmdarg {
    fn default() -> Self {
        Self::new()
    }
}

const K_CPO_REDO: i32 = b'r' as i32;

pub const K_ACOMP: i32 = 0;
pub const K_AEXCMD: i32 = 1;
pub const K_AINSERT: i32 = 2;
pub const K_AINSERT_SPAN: i32 = 3;
pub const K_AJUMP: i32 = 4;
pub const K_AMOTION: i32 = 5;
pub const K_AMOUSE: i32 = 6;
pub const K_ANORMAL: i32 = 7;
pub const K_AOPERATOR: i32 = 8;
pub const K_ASCROLL: i32 = 9;
pub const K_AVISUAL: i32 = 10;
pub const K_AVISUAL_SPAN: i32 = 11;
pub type CmdAtomType = i32;

pub const K_VINS_NONE: i32 = 0;
pub const K_VINS_KEYS: i32 = 1;
pub const K_VINS_MOTION: i32 = 2;
pub const K_VINS_OTHER: i32 = 3;
pub type VisualIns = i32;

pub const K_KEY_OPAQUE: u32 = 1 << 0;
pub const K_KEY_SYNTHETIC: u32 = 1 << 1;
pub const K_KEY_PAYLOAD: u32 = 1 << 2;
pub const K_KEY_SCROLL_MOVE: u32 = 1 << 3;
pub const K_KEY_SCROLL_VIEW: u32 = 1 << 4;
pub const K_KEY_JUMP: u32 = 1 << 5;
pub const K_KEY_MOTION: u32 = 1 << 6;
pub const K_KEY_INS_FLUSH: u32 = 1 << 7;
pub const K_KEY_MOUSE: u32 = 1 << 8;

const K_VATOM_NONE: u32 = 0;
const K_VATOM_TYPED: u32 = 1;
const K_VATOM_FED: u32 = 2;
const K_VATOM_VOID: u32 = 4;

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
    pub count: i64,
    pub regname: i32,
    pub op: i32,
    pub op_extra: i32,
    pub motion_force: i32,
    pub cmd: i32,
    pub cmd2: i32,
    pub cmdarg: i32,
    pub body: Vec<u8>,
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
    pub buf: Bufref,
    pub win: crate::globals::WinPtr,
    pub pos: CursorPos,
    pub mcursor: u32,
    pub tick: Varnumber,
    pub maptick: i32,
}

impl CmdOrigin {
    pub const fn new() -> Self {
        Self {
            buf: Bufref {
                br_buf: crate::globals::BufPtr::NULL,
                br_fnum: 0,
                br_buf_free_count: 0,
            },
            win: crate::globals::WinPtr::NULL,
            pos: CursorPos {
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
    pub undoseq: i32,
    pub changed: bool,
    pub moved: bool,
    pub remap: bool,
    pub cascaded: bool,
}

impl CmdAtom {
    pub const fn new() -> Self {
        Self {
            r#type: K_ACOMP,
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
    pub keyclass: u32,
    pub ex_normal: i32,
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
    active: std::cell::Cell<bool>,
    open: std::cell::Cell<bool>,
    origin: std::cell::Cell<CmdOrigin>,
    follow: std::cell::Cell<TriState>,
    atoms: std::cell::RefCell<CmdAtomVec>,
    lhs: std::cell::Cell<[u8; MAXMAPLEN + 4]>,
    queued: std::cell::Cell<bool>,
    lossy: std::cell::Cell<bool>,
    r#macro: std::cell::Cell<bool>,
    frame: std::cell::Cell<u64>,
}

struct VatomStateStruct {
    atoms: std::cell::RefCell<CmdAtomVec>,
    state: std::cell::Cell<u32>,
    origin: std::cell::Cell<CmdOrigin>,
    frame: std::cell::Cell<u64>,
    done: std::cell::Cell<usize>,
}

struct TypedState {
    keys: std::cell::RefCell<Vec<u8>>,
    map_start: std::cell::Cell<usize>,
}

pub(crate) struct CmdAtomModuleState {
    pub(crate) g_atoms: std::cell::RefCell<CmdAtomVec>,
    atom_captures: std::cell::Cell<u64>,
    global_ops: std::cell::Cell<u64>,
    atom_suppressed: std::cell::Cell<bool>,
    map_edit: std::cell::Cell<bool>,
    frame_id: std::cell::Cell<u64>,
    cmd_frames: std::cell::RefCell<Vec<CmdFrame>>,
    composite: CompositeState,
    vatom: VatomStateStruct,
    typed: TypedState,
}

impl CmdAtomModuleState {
    const fn new() -> Self {
        Self {
            g_atoms: std::cell::RefCell::new(Vec::new()),
            atom_captures: std::cell::Cell::new(0),
            global_ops: std::cell::Cell::new(0),
            atom_suppressed: std::cell::Cell::new(false),
            map_edit: std::cell::Cell::new(false),
            frame_id: std::cell::Cell::new(0),
            cmd_frames: std::cell::RefCell::new(Vec::new()),
            composite: CompositeState {
                active: std::cell::Cell::new(false),
                open: std::cell::Cell::new(false),
                origin: std::cell::Cell::new(CmdOrigin::new()),
                follow: std::cell::Cell::new(K_NONE),
                atoms: std::cell::RefCell::new(Vec::new()),
                lhs: std::cell::Cell::new([0; MAXMAPLEN + 4]),
                queued: std::cell::Cell::new(false),
                lossy: std::cell::Cell::new(false),
                r#macro: std::cell::Cell::new(false),
                frame: std::cell::Cell::new(0),
            },
            vatom: VatomStateStruct {
                atoms: std::cell::RefCell::new(Vec::new()),
                state: std::cell::Cell::new(K_VATOM_NONE),
                origin: std::cell::Cell::new(CmdOrigin::new()),
                frame: std::cell::Cell::new(0),
                done: std::cell::Cell::new(0),
            },
            typed: TypedState {
                keys: std::cell::RefCell::new(Vec::new()),
                map_start: std::cell::Cell::new(0),
            },
        }
    }
}

thread_local! {
    static ATOM_STATE: CmdAtomModuleState = const { CmdAtomModuleState::new() };
}

#[inline]
pub(crate) fn with_atom<R>(f: impl FnOnce(&CmdAtomModuleState) -> R) -> R {
    ATOM_STATE.with(f)
}

#[inline]
pub fn sb_concat(sb: &mut Vec<u8>, s: &[u8]) {
    sb.extend_from_slice(crate::memory::bytes_until_nul(s));
}

#[inline]
fn dict_put(d: &mut Dict, key: &CStr, value: Object) {
    d.push(KeyValuePair {
        key: cstr_to_string(Some(key)),
        value,
    });
}

use crate::api::private::helpers::array_add;

#[inline]
fn with_cur_frame<R>(f: impl FnOnce(&CmdFrame) -> R) -> R {
    with_atom(|st| f(st.cmd_frames.borrow().last().unwrap()))
}

#[inline]
fn with_cur_frame_opt<R>(f: impl FnOnce(Option<&CmdFrame>) -> R) -> R {
    with_atom(|st| f(st.cmd_frames.borrow().last()))
}

#[inline]
fn with_cur_frame_mut<R>(f: impl FnOnce(&mut CmdFrame) -> R) -> R {
    with_atom(|st| f(st.cmd_frames.borrow_mut().last_mut().unwrap()))
}

#[inline]
fn with_root_frame<R>(f: impl FnOnce(&CmdFrame) -> R) -> R {
    with_atom(|st| f(&st.cmd_frames.borrow()[0]))
}

#[inline]
fn with_root_frame_mut<R>(f: impl FnOnce(&mut CmdFrame) -> R) -> R {
    with_atom(|st| f(&mut st.cmd_frames.borrow_mut()[0]))
}

#[inline]
fn is_child_frame() -> bool {
    with_atom(|st| {
        let frames = st.cmd_frames.borrow();
        if frames.len() <= 1 {
            return false;
        }
        let root = &frames[0];
        let cur = frames.last().unwrap();
        let fed = cur.ex_normal > root.ex_normal;
        fed && (root.keyclass & K_KEY_SYNTHETIC) == 0
    })
}

pub fn atom_free_all() {
    with_atom(|st| {
        *st.g_atoms.borrow_mut() = Vec::new();
        for frame in st.cmd_frames.borrow_mut().iter_mut().rev() {
            frame.staged = CmdAtom::new();
            frame.cmdline = None;
        }
    });
    atom_composite_abort();
    with_atom(|st| {
        *st.composite.atoms.borrow_mut() = Vec::new();
        *st.typed.keys.borrow_mut() = Vec::new();
        *st.vatom.atoms.borrow_mut() = Vec::new();
    });
}

pub fn atom_cmd_spec(cap: &Cmdarg) -> CmdSpec {
    let operand = nv_is(cap.cmdchar, NV_LANG | NV_NCH_ARG);
    CmdSpec {
        regname: cap.oap_mut().regname,
        count: i64::from(cap.count0),
        cmd: cap.cmdchar,
        cmd2: if operand { NUL } else { cap.nchar },
        cmdarg: if operand { cap.nchar } else { cap.extra_char },
        ..CmdSpec::default()
    }
}

fn atom_origin() -> CmdOrigin {
    let mut origin = CmdOrigin {
        win: crate::globals::curwin_ptr(),
        pos: curwin_mut().w_cursor,
        tick: buf_get_changedtick(curbuf_mut()),
        maptick: g_st().maptick,
        mcursor: mc_mark_at(curbuf_mut(), curwin_mut().w_cursor, 0),
        ..CmdOrigin::default()
    };
    set_bufref(&mut origin.buf, g_st().curbuf.opt_mut());
    origin
}

#[inline]
fn atom_origin_changed(origin: CmdOrigin) -> bool {
    bufref_valid(Some(&origin.buf))
        && origin
            .buf
            .buf_opt()
            .is_some_and(|b| buf_get_changedtick(b) != origin.tick)
}

#[inline]
fn atom_origin_moved(origin: CmdOrigin) -> bool {
    bufref_valid(Some(&origin.buf))
        && win_valid(origin.win.opt())
        && origin
            .win
            .opt()
            .is_some_and(|w| origin.buf.br_buf == w.w_buffer && !equalpos(origin.pos, w.w_cursor))
}

#[inline]
fn atom_origin_undoseq(origin: CmdOrigin) -> i32 {
    if bufref_valid(Some(&origin.buf)) {
        origin.buf.buf_opt().map_or(0, |b| b.b_u_seq_cur)
    } else {
        0
    }
}

pub fn atom_origin_pos(buf: &mut FileBuf) -> CursorPos {
    let (o, open) = with_atom(|st| (st.composite.origin.get(), st.composite.open.get()));
    if open
        && o.buf.br_buf == (&raw mut *buf)
        && bufref_valid(Some(&o.buf))
        && buf_get_changedtick(buf) == o.tick
    {
        return o.pos;
    }
    curwin_mut().w_cursor
}

fn atom_redo_keys(spec: &CmdSpec) -> std::ffi::CString {
    assert!(!mc_replaying());
    redo_keys(Some(spec)).expect("atom_redo_keys must be non-empty")
}

fn atom_from_redo(r#type: CmdAtomType) -> CmdAtom {
    let keys_cstr = redo_keys(None);
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
    ca: &Cmdarg,
    cmdline: Option<&CStr>,
    vis: bool,
) -> CmdAtom {
    let cmdline_bytes = cmdline.map_or(&[][..], CStr::to_bytes);
    let mut sb: Vec<u8> = Vec::new();
    if ca.cmdchar != b':' as i32 && ca.count0 != 0 {
        let mut num_buf = [0u8; 32];
        let n = format_to_buf!(&mut num_buf, c"%d", ca.count0);
        if n > 0 {
            sb.extend_from_slice(&num_buf[..n as usize]);
        }
    }
    sb_add_char(&mut sb, ca.cmdchar);
    if vis && ca.cmdchar == b':' as i32 {
        sb_add_char(&mut sb, CTRL_U);
    }
    if is_special(ca.cmdchar) {
        sb_add_spec(&mut sb, cmdline_bytes);
    } else {
        sb_add_lit(&mut sb, cmdline_bytes);
    }
    sb_add_char(&mut sb, NL);
    let mut atom = CmdAtom::new();
    atom.r#type = r#type;
    atom.spec = CmdSpec {
        count: i64::from(ca.count0),
        cmd: ca.cmdchar,
        ..CmdSpec::default()
    };
    atom.keys = Some(std::ffi::CString::new(sb).unwrap());
    atom.text = cmdline
        .map(CStr::to_owned)
        .or_else(|| Some(std::ffi::CString::default()));
    atom
}

fn atom_from_cur_frame(ca: &mut Cmdarg) -> CmdAtom {
    let (root_cmdline, visual_active) = with_atom(|st| {
        let frames = st.cmd_frames.borrow();
        (
            frames[0].cmdline.clone(),
            frames.last().unwrap().visual.active,
        )
    });
    let mut atom = CmdAtom::default();
    if let Some(ref cmdline) = root_cmdline
        && (ca.cmdchar == b':' as i32 || ca.cmdchar == K_COMMAND)
    {
        atom = atom_from_cmdline(K_AEXCMD, ca, Some(cmdline), visual_active);
    } else if ca.cmdchar == K_LUA {
        let mut lua_id = [0u8; NUMBUFLEN];
        format_to_buf!(&mut lua_id, c"%d", g_st().repeat_luamap);
        atom = atom_from_cmdline(
            K_ANORMAL,
            ca,
            CStr::from_bytes_until_nul(&lua_id).ok(),
            visual_active,
        );
    }
    atom_payload_append_cur_frame(&mut atom);
    atom
}

fn atoms_concat_keys(atoms: &[CmdAtom], from: usize) -> std::ffi::CString {
    let mut keys: Vec<u8> = Vec::new();
    for atom in &atoms[from..] {
        if let Some(ref k) = atom.keys {
            keys.extend_from_slice(k.as_bytes());
        }
    }
    bytes_to_cstring(&keys)
}

fn atom_key_name(c: i32) -> std::ffi::CString {
    if c == NUL || c == K_LUA {
        return std::ffi::CString::default();
    }
    if is_special(c) || c < b' ' as i32 {
        return get_special_key_name(c, 0).as_cstr().to_owned();
    }
    if c == DEL {
        return c"<Del>".to_owned();
    }
    let mut buf = [0u8; MB_MAXBYTES + 1];
    let n = utf_char2bytes(c, &mut buf) as usize;
    buf[n] = 0;
    bytes_to_cstring(&buf[..n])
}

fn atom_dict(atom: &CmdAtom) -> Dict {
    let spec = &atom.spec;
    let regname: [u8; 2] = [spec.regname as u8, 0];
    let force_buf: [u8; 2] = [spec.motion_force as u8, 0];
    let force: &CStr = if spec.motion_force == CTRL_V_CONST {
        c"<C-V>"
    } else {
        CStr::from_bytes_until_nul(&force_buf).unwrap_or(c"")
    };

    let mut op_bytes = atom_key_name(spec.op).into_bytes();
    if spec.op_extra != NUL {
        op_bytes.extend_from_slice(atom_key_name(spec.op_extra).as_bytes());
    }
    let op = bytes_to_cstring(&op_bytes);

    let mut cmd_bytes = atom_key_name(spec.cmd).into_bytes();
    if spec.cmd2 != NUL {
        cmd_bytes.extend_from_slice(atom_key_name(spec.cmd2).as_bytes());
    }
    let cmd = bytes_to_cstring(&cmd_bytes);

    let mut d = Dict {
        items: crate::globals::GraphPtr::NULL,
        size: 0,
        capacity: 0,
    };
    let cmdarg = atom_key_name(spec.cmdarg);
    if !cmdarg.as_bytes().is_empty() {
        dict_put(&mut d, c"cmdarg", cstr_to_obj(Some(cmdarg.as_c_str())));
    }
    dict_put(&mut d, c"changed", boolean_obj(atom.changed));
    if !cmd.as_bytes().is_empty() {
        dict_put(&mut d, c"cmd", cstr_to_obj(Some(cmd.as_c_str())));
    }
    if spec.count > 0 {
        dict_put(&mut d, c"count", integer_obj(spec.count as _));
    }
    if atom.origin.pos.lnum > 0 {
        dict_put(&mut d, c"moved", boolean_obj(atom.moved));
        let mut pos = Array {
            items: crate::globals::GraphPtr::NULL,
            size: 0,
            capacity: 0,
        };
        array_add(&mut pos, integer_obj(i64::from(atom.origin.pos.lnum)));
        array_add(&mut pos, integer_obj(i64::from(atom.origin.pos.col)));
        dict_put(&mut d, c"pos", array_obj(pos));
        dict_put(&mut d, c"undoseq", integer_obj(i64::from(atom.undoseq)));
    }
    let keys: &CStr = atom.keys.as_deref().unwrap_or(c"");
    if !atom.remap {
        dict_put(&mut d, c"keys", cstr_to_obj(Some(keys)));
    }
    let lhs_cstr = match &atom.lhs {
        Some(lhs) if !lhs.as_bytes().is_empty() => lhs.as_c_str(),
        _ => keys,
    };
    dict_put(&mut d, c"lhs", cstr_to_obj(Some(lhs_cstr)));
    if !force.to_bytes().is_empty() {
        dict_put(&mut d, c"motionforce", cstr_to_obj(Some(force)));
    }
    if !op.as_bytes().is_empty() {
        dict_put(&mut d, c"operator", cstr_to_obj(Some(op.as_c_str())));
    }
    if spec.regname != 0 {
        dict_put(
            &mut d,
            c"reg",
            cstr_to_obj(Some(crate::memory::bytes_as_cstr(&regname))),
        );
    }
    if let Some(ref text) = atom.text
        && !text.as_bytes().is_empty()
    {
        dict_put(&mut d, c"text", cstr_to_obj(Some(text.as_c_str())));
    }
    dict_put(
        &mut d,
        c"type",
        cstr_to_obj(Some(TYPE_NAMES[atom.r#type as usize])),
    );
    d
}

fn atom_emit(atom: &CmdAtom) {
    if !has_event(EVENT_CMDATOM) {
        return;
    }
    let mut data = atom_dict(atom);
    if !atom.atoms.is_empty() {
        let mut atoms = Array {
            items: crate::globals::GraphPtr::NULL,
            size: 0,
            capacity: 0,
        };
        for sub in &atom.atoms {
            array_add(&mut atoms, dict_obj(atom_dict(sub)));
        }
        dict_put(&mut data, c"atoms", array_obj(atoms));
    }
    let buf = atom.origin.buf.buf_opt().unwrap_or_else(curbuf_mut);
    let mut obj = dict_obj(data);
    aucmd_defer(
        EVENT_CMDATOM,
        Some(TYPE_NAMES[atom.r#type as usize]),
        None,
        AUGROUP_ALL,
        buf,
        Some(&mut obj),
    );
    api_free_dict(data);
}

pub fn atom_push_raw(cascade: bool, atom: &mut CmdAtom) {
    assert!(atom.keys.is_some());
    if !atom.origin.buf.br_buf.is_null() {
        atom.changed = atom_origin_changed(atom.origin);
        atom.moved = atom_origin_moved(atom.origin);
        atom.undoseq = atom_origin_undoseq(atom.origin);
    }
    let cur_frame_id = with_cur_frame_opt(|f| f.map_or(0, |f| f.id));
    let collect = with_atom(|st| {
        st.composite.active.get() && (cur_frame_id == 0 || cur_frame_id != st.composite.frame.get())
    });
    if atom_visual_pending() {
        with_atom(|st| {
            if (st.vatom.state.get() & K_VATOM_TYPED) != 0 {
                st.atom_captures.set(st.atom_captures.get() + 1);
            }
            if collect {
                st.composite.queued.set(true);
            }
            st.vatom.atoms.borrow_mut().push(std::mem::take(atom));
            st.vatom.frame.set(cur_frame_id);
        });
        return;
    }
    with_atom(|st| st.atom_captures.set(st.atom_captures.get() + 1));
    if atom.r#type == K_AVISUAL
        && let Some(last) = atom.atoms.last_mut()
        && last.r#type == K_AOPERATOR
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
        with_atom(|st| {
            st.g_atoms.borrow_mut().push(copy);
            if collect {
                st.composite.queued.set(true);
            }
        });
    }
    if collect {
        with_atom(|st| st.composite.atoms.borrow_mut().push(std::mem::take(atom)));
    } else {
        if atom.r#type != K_AINSERT_SPAN {
            atom_emit(&*atom);
        }
        *atom = CmdAtom::new();
    }
}

fn atom_push(cascade: bool, atom: &mut CmdAtom) {
    if atom_blocked() {
        *atom = CmdAtom::new();
        return;
    }
    atom_push_raw(cascade && mc_buf_has_cursors(curbuf_mut()), &mut *atom);
}

fn atom_stage_set(atom: &mut CmdAtom) {
    assert!(!atom_staged());
    with_cur_frame_mut(|f| f.staged = CmdAtom::new());
    if atom_blocked() {
        *atom = CmdAtom::new();
        return;
    }
    assert!(atom.keys.is_some());
    with_cur_frame_mut(|f| f.staged = std::mem::take(&mut *atom));
}

#[inline]
fn atom_staged() -> bool {
    with_cur_frame_opt(|f| f.is_some_and(|f| f.staged.keys.is_some()))
}

fn atom_stage_flush() {
    let Some((mut staged, frame_global_ops)) = with_atom(|st| {
        let mut frames = st.cmd_frames.borrow_mut();
        let frame = frames.last_mut()?;
        frame
            .staged
            .keys
            .is_some()
            .then(|| (std::mem::take(&mut frame.staged), frame.global_ops))
    }) else {
        return;
    };
    let cascade = !staged.keys.as_ref().unwrap().as_bytes().is_empty()
        && !staged.cascaded
        && with_atom(|st| st.global_ops.get()) == frame_global_ops;
    atom_push(cascade, &mut staged);
}

fn atom_composite_lhs() -> std::ffi::CString {
    let mut keys: Vec<u8> = Vec::new();
    with_atom(|st| {
        let lhs = st.composite.lhs.get();
        sb_concat(&mut keys, &lhs);
        let typed_keys = st.typed.keys.borrow();
        let tail = &typed_keys[st.typed.map_start.get()..];
        keys.extend_from_slice(tail);
    });
    bytes_to_cstring(&keys)
}

pub fn atom_lhs_replay_queue() {
    let mut atom = CmdAtom::new();
    atom.r#type = K_ACOMP;
    atom.keys = Some(atom_composite_lhs());
    atom.remap = true;
    with_atom(|st| {
        atom.origin = st.composite.origin.get();
        st.g_atoms.borrow_mut().push(atom);
    });
}

pub fn atom_composite_queued() -> bool {
    with_atom(|st| st.composite.queued.get())
}

pub fn atom_composite_active() -> bool {
    with_atom(|st| st.composite.active.get())
}

fn atom_composite_label(lhs: &[u8]) {
    let copy_len = lhs.len().min(MAXMAPLEN + 3);
    with_atom(|st| {
        let mut buf = st.composite.lhs.get();
        buf[..copy_len].copy_from_slice(&lhs[..copy_len]);
        buf[copy_len] = 0;
        st.composite.lhs.set(buf);
    });
}

fn atom_composite_start(lhs: &[u8]) {
    if !lhs.is_empty() {
        atom_composite_label(lhs);
    }
    with_atom(|st| {
        st.composite.active.set(true);
        st.composite.queued.set(false);
        st.composite.lossy.set(false);
        st.composite.frame.set(0);
    });
}

fn atom_composite_end() {
    let is_active = with_atom(|st| {
        st.composite.r#macro.set(false);
        st.composite.active.get()
    });
    if !is_active {
        return;
    }
    let remap = with_atom(|st| st.composite.lossy.get() || st.composite.atoms.borrow().is_empty());
    let lhs = atom_composite_lhs();
    let (single_atom, keys_cstr, origin, sub_atoms) = with_atom(|st| {
        let mut buf = st.composite.lhs.get();
        buf[0] = 0;
        st.composite.lhs.set(buf);
        st.composite.active.set(false);
        let mut atoms = st.composite.atoms.borrow_mut();
        if atoms.len() == 1 {
            (
                Some(atoms.pop().unwrap()),
                None,
                CmdOrigin::new(),
                Vec::new(),
            )
        } else {
            let keys_cstr = atoms_concat_keys(&atoms, 0);
            let sub_atoms = std::mem::take(&mut *atoms);
            (None, Some(keys_cstr), st.composite.origin.get(), sub_atoms)
        }
    });
    let mut atom: CmdAtom;
    if let Some(single) = single_atom {
        atom = single;
        atom.lhs = Some(lhs);
        atom.remap = remap;
    } else {
        atom = CmdAtom::new();
        atom.r#type = K_ACOMP;
        atom.keys = keys_cstr;
        atom.lhs = Some(lhs);
        atom.remap = remap;
        atom.origin = origin;
        atom.changed = atom_origin_changed(origin);
        atom.moved = atom_origin_moved(origin);
        atom.undoseq = atom_origin_undoseq(origin);
        atom.atoms = sub_atoms;
    }
    atom_emit(&atom);
}

pub fn atom_composite_abort() {
    with_atom(|st| {
        st.composite.r#macro.set(false);
        let mut buf = st.composite.lhs.get();
        buf[0] = 0;
        st.composite.lhs.set(buf);
        st.composite.active.set(false);
        *st.composite.atoms.borrow_mut() = Vec::new();
        st.composite.open.set(false);
    });
}

pub fn atom_term_enter() {
    if !mc_replaying() {
        atom_composite_end();
        with_atom(|st| st.composite.open.set(false));
    }
}

pub fn atom_is_user_cmd() -> bool {
    !mc_replaying()
        && ((g_st().reg_executing == 0 && !g_st().pending_end_reg_executing)
            || with_atom(|st| st.composite.r#macro.get()))
        && g_st().ex_normal_busy == 0
}

#[inline]
fn atom_is_user_input() -> bool {
    g_st().key_typed || (atom_is_user_cmd() && atom_composite_active())
}

pub fn atom_suppress(suppress: bool) {
    with_atom(|st| st.atom_suppressed.set(suppress));
}

#[inline]
fn atom_blocked() -> bool {
    mc_replaying() || with_atom(|st| st.atom_suppressed.get())
}

#[inline]
fn atom_capturable(consumers: bool, keytyped: bool) -> bool {
    consumers && atom_is_user_cmd() && (keytyped || atom_composite_active())
}

#[inline]
fn atom_buf_has_consumers() -> bool {
    mc_buf_has_cursors(curbuf_mut()) || has_event(EVENT_CMDATOM)
}

pub fn atom_key_class(cmd: i32, arg: i32, arg2: i32) -> u32 {
    if cmd == K_EVENT || cmd == K_IGNORE {
        return K_KEY_OPAQUE | K_KEY_SYNTHETIC;
    }
    if cmd == K_COMMAND || cmd == K_LUA {
        return K_KEY_OPAQUE;
    }
    if cmd == b'/' as i32 || cmd == b'?' as i32 || cmd == b':' as i32 || cmd == b'!' as i32 {
        return K_KEY_PAYLOAD;
    }
    if cmd == CTRL_D_CONST || cmd == CTRL_U {
        return K_KEY_SCROLL_MOVE | K_KEY_INS_FLUSH;
    }
    if cmd == CTRL_F_CONST || cmd == CTRL_B_CONST {
        return K_KEY_SCROLL_MOVE;
    }
    if cmd == CTRL_E || cmd == CTRL_Y {
        return K_KEY_SCROLL_VIEW;
    }
    if cmd == CTRL_T_CONST {
        return K_KEY_INS_FLUSH;
    }
    if cmd == b'g' as i32 {
        if arg == b'\'' as i32 || arg == b'`' as i32 {
            return atom_key_class(arg, arg2, NUL);
        }
        if b";,go".contains(&(arg as u8)) {
            return K_KEY_JUMP;
        }
        return if b"jk0^$_meEM*#".contains(&(arg as u8)) {
            K_KEY_MOTION
        } else {
            0
        };
    }
    if cmd == b'[' as i32 || cmd == b']' as i32 {
        if arg == b'C' as i32 {
            return K_KEY_JUMP;
        }
        return if b"[](){}mMcsz#*/".contains(&(arg as u8)) {
            K_KEY_MOTION
        } else {
            0
        };
    }
    if cmd == b'z' as i32 {
        return if arg == b'j' as i32 || arg == b'k' as i32 {
            K_KEY_MOTION
        } else {
            0
        };
    }
    if cmd == b'\'' as i32 || cmd == b'`' as i32 {
        return if arg != NUL && b"<>[].^(){}".contains(&(arg as u8)) {
            K_KEY_MOTION
        } else {
            K_KEY_JUMP
        };
    }
    if cmd == K_DOWN
        || cmd == K_END
        || cmd == K_HOME
        || cmd == K_LEFT
        || cmd == K_RIGHT
        || cmd == K_UP
    {
        return K_KEY_MOTION | K_KEY_INS_FLUSH;
    }
    if cmd == CTRL_G
        || cmd == CTRL_H_CONST
        || cmd == CTRL_R_CONST
        || cmd == CTRL_W_CONST
        || cmd == K_BS
        || cmd == K_DEL
        || cmd == K_S_LEFT
        || cmd == K_S_RIGHT
    {
        return K_KEY_INS_FLUSH;
    }
    if cmd == K_LEFTMOUSE
        || cmd == K_LEFTMOUSE_NM
        || cmd == K_MIDDLEMOUSE
        || cmd == K_RIGHTMOUSE
        || cmd == K_X1MOUSE
        || cmd == K_X2MOUSE
    {
        return K_KEY_MOUSE;
    }
    if cmd == K_MOUSEDOWN || cmd == K_MOUSEUP || cmd == K_MOUSELEFT || cmd == K_MOUSERIGHT {
        return K_KEY_SCROLL_VIEW;
    }
    0
}

pub fn atom_cmdline_set(firstc: i32, line: &[u8]) {
    if with_cur_frame_opt(|f| f.is_none())
        || !atom_is_user_cmd()
        || (firstc != b':' as i32 && firstc != K_COMMAND)
    {
        return;
    }
    with_root_frame_mut(|root| {
        if root.cmdline.is_none() {
            root.cmdline = Some(std::ffi::CString::new(line).unwrap());
        }
    });
}

#[inline]
fn atom_typed_collecting() -> bool {
    atom_composite_active()
        || with_cur_frame_opt(|f| f.is_some_and(|f| f.payload_start != usize::MAX))
}

pub fn atom_payload_start() {
    if g_st().expr_map_lock == 0 && !mc_replaying() {
        with_atom(|st| {
            let mut frames = st.cmd_frames.borrow_mut();
            if let Some(cur_frame) = frames.last_mut()
                && cur_frame.payload_start == usize::MAX
            {
                let len = st.typed.keys.borrow().len();
                cur_frame.payload_start = len;
                cur_frame.payload_end = len;
            }
        });
    }
}

pub fn atom_payload_end() {
    if g_st().expr_map_lock == 0 && !mc_replaying() {
        with_atom(|st| {
            let mut frames = st.cmd_frames.borrow_mut();
            if let Some(cur_frame) = frames.last_mut()
                && cur_frame.payload_start != usize::MAX
            {
                cur_frame.payload_end = st.typed.keys.borrow().len();
            }
        });
    }
}

fn atom_payload_append_cur_frame(atom: &mut CmdAtom) {
    let Some(ref cur_keys) = atom.keys else {
        return;
    };
    let mut bytes = cur_keys.as_bytes().to_vec();
    let appended = with_atom(|st| {
        let mut frames = st.cmd_frames.borrow_mut();
        let frame = frames.last_mut().unwrap();
        let plen = if frame.payload_start == usize::MAX {
            0
        } else {
            frame.payload_end - frame.payload_start
        };
        if plen == 0 {
            return false;
        }
        bytes.extend_from_slice(&st.typed.keys.borrow()[frame.payload_start..frame.payload_end]);
        frame.payload_start = usize::MAX;
        true
    });
    if appended {
        atom.keys = Some(std::ffi::CString::new(bytes).unwrap());
    }
}

pub fn atom_typed_add(slice: &[u8]) {
    if mc_replaying() || !atom_typed_collecting() || g_st().cmdline_star > 0 || slice.is_empty() {
        return;
    }
    if slice.len() == 3
        && slice[0] == K_SPECIAL
        && (atom_key_class(
            termcap2key(i32::from(slice[1]), i32::from(slice[2])),
            NUL,
            NUL,
        ) & K_KEY_SYNTHETIC)
            != 0
    {
        return;
    }
    with_atom(|st| st.typed.keys.borrow_mut().extend_from_slice(slice));
}

pub fn atom_typed_del(len: usize) {
    if mc_replaying() || !atom_typed_collecting() {
        return;
    }
    with_atom(|st| {
        let mut keys = st.typed.keys.borrow_mut();
        let new_len = keys.len().saturating_sub(len);
        keys.truncate(new_len);
    });
}

pub fn atom_did_global_op() {
    if !mc_replaying() {
        with_atom(|st| {
            st.global_ops.set(st.global_ops.get() + 1);
            st.composite.queued.set(true);
        });
    }
}

pub fn atom_follow_changed() {
    if mc_replaying() {
        return;
    }
    let following = mc_following();
    with_atom(|st| {
        for frame in st.cmd_frames.borrow_mut().iter_mut().rev() {
            if !atom_origin_moved(frame.origin) {
                frame.follow = following;
            } else if st.composite.follow.get() == K_NONE {
                st.composite
                    .follow
                    .set(if frame.follow { K_TRUE } else { K_FALSE });
            }
        }
    });
}

pub fn atom_redo_prepped() {
    if atom_is_user_cmd() {
        let cur_id = with_cur_frame(|f| f.id);
        with_root_frame_mut(|root| root.redo_frame = cur_id);
    }
}

pub fn atom_redo_cancel() {
    with_root_frame_mut(|root| root.redo_frame = 0);
}

pub fn atom_macro_start(regname: i32) {
    if atom_is_user_input() {
        with_atom(|st| st.composite.r#macro.set(true));
        if !atom_composite_active() {
            let lhs: [u8; 2] = [b'@', regname as u8];
            atom_composite_start(&lhs);
        }
    }
}

pub fn atom_stuff_start(cap: &Cmdarg) {
    if mc_replaying() || atom_composite_active() || !atom_is_user_input() {
        return;
    }
    let lhs = atom_redo_keys(&atom_cmd_spec(cap));
    atom_composite_start(lhs.as_bytes());
}

pub fn atom_map_start(lhs: &[u8], peeked: bool) {
    if mc_replaying() || g_st().reg_executing != 0 || g_st().ex_normal_busy != 0 {
        return;
    }
    if (g_st().state_global & MODE_NORMAL) == 0 {
        atom_composite_label(lhs);
        return;
    }
    if atom_composite_active() {
        if get_real_state() == MODE_OP_PENDING {
            return;
        }
        with_atom(|st| st.typed.map_start.set(st.typed.keys.borrow().len()));
        atom_composite_end();
    }
    let op = get_vim_var_str(VV_OP);
    if get_real_state() == MODE_OP_PENDING && !op.is_empty() {
        let mut full_bytes = Vec::from(op.to_bytes());
        full_bytes.extend_from_slice(lhs);
        atom_composite_start(&full_bytes);
        with_atom(|st| st.typed.map_start.set(st.typed.keys.borrow().len()));
        return;
    }
    atom_composite_start(lhs);
    with_atom(|st| {
        if peeked && let Some(cur_frame) = st.cmd_frames.borrow().last() {
            st.composite.frame.set(cur_frame.id);
        }
        st.typed.map_start.set(st.typed.keys.borrow().len());
    });
}

pub fn atom_visual_reset() {
    with_atom(|st| {
        st.vatom.state.set(K_VATOM_NONE);
        *st.vatom.atoms.borrow_mut() = Vec::new();
        st.vatom.origin.set(CmdOrigin::default());
        st.vatom.frame.set(0);
        st.vatom.done.set(0);
    });
    mc_vsel_clear();
}

#[inline]
fn atom_visual_pending() -> bool {
    with_atom(|st| st.vatom.state.get() != K_VATOM_NONE)
}

#[inline]
fn atom_visual_typed() -> bool {
    atom_visual_pending()
        && with_atom(|st| {
            (st.vatom.state.get() & K_VATOM_TYPED) != 0
                || g_st().maptick != st.vatom.origin.get().maptick
        })
}

pub fn atom_visual_replayable() -> bool {
    atom_visual_pending() && with_atom(|st| (st.vatom.state.get() & K_VATOM_VOID) == 0)
}

pub fn atom_visual_redoable() -> bool {
    if !atom_visual_replayable() {
        return false;
    }
    with_atom(|st| {
        for atom in st.vatom.atoms.borrow().iter() {
            if atom.spec.cmd == b'g' as i32 && atom.spec.cmd2 == b'v' as i32 {
                return false;
            }
        }
        true
    })
}

pub fn atom_visual_span() -> Option<std::ffi::CString> {
    if !atom_visual_replayable() {
        return None;
    }
    let (tail, done) = with_atom(|st| {
        let done = st.vatom.done.get();
        let tail = atoms_concat_keys(&st.vatom.atoms.borrow(), done);
        (tail, done)
    });
    if done == 0 {
        return Some(tail);
    }

    let mut keys: Vec<u8> = Vec::with_capacity(2 + tail.as_bytes().len());
    keys.extend_from_slice(b"gv");
    keys.extend_from_slice(tail.as_bytes());
    Some(bytes_to_cstring(&keys))
}

fn atom_visual_span_flush() {
    if with_atom(|st| st.vatom.done.get() >= st.vatom.atoms.borrow().len())
        || !atom_visual_replayable()
        || !atom_visual_typed()
        || !mc_buf_has_cursors(curbuf_mut())
    {
        return;
    }
    let edited = with_atom(|st| {
        let mut edited = false;
        let atoms = st.vatom.atoms.borrow();
        for atom in &atoms[st.vatom.done.get()..] {
            assert!(atom.r#type != K_AOPERATOR);
            edited |= atom.changed;
        }
        edited
    });
    if !edited {
        return;
    }
    let span_cstr = atom_visual_span();
    with_atom(|st| {
        let len = st.vatom.atoms.borrow().len();
        st.vatom.done.set(len);
        let mut atom = CmdAtom::new();
        atom.r#type = K_AVISUAL_SPAN;
        atom.keys = span_cstr;
        atom.origin = st.vatom.origin.get();
        st.g_atoms.borrow_mut().push(atom);
    });
}

fn atom_visual_end_suffix(
    suffix: Option<std::ffi::CString>,
    spec: Option<&CmdSpec>,
    redoable: bool,
) -> bool {
    if mc_replaying() || with_atom(|st| st.atom_suppressed.get()) {
        return false;
    }
    let origin = with_atom(|st| st.vatom.origin.get());
    let replayable = suffix.is_some() && atom_visual_replayable();
    let v = if replayable {
        Some(with_atom(|st| {
            atoms_concat_keys(&st.vatom.atoms.borrow(), 0)
        }))
    } else {
        None
    };
    let prepped =
        redoable && spec.is_some() && suffix.is_some() && (replayable || spec.unwrap().op != NUL);

    if prepped {
        let use_redo_keys = atom_visual_redoable();
        let redo_bytes = if use_redo_keys && let Some(ref vc) = v {
            vc.as_bytes()
        } else {
            b"1v"
        };
        prep_redo_visual(redo_bytes, CmdSpec::default());
        if let Some(ref suf) = suffix {
            redo_append_str(suf.as_bytes());
        }
    }

    if is_child_frame() {
        return prepped;
    }

    if !replayable {
        let emit = with_atom(|st| (st.vatom.state.get() & K_VATOM_VOID) != 0)
            && atom_visual_typed()
            && spec.is_some()
            && suffix.is_some()
            && atom_is_user_cmd();
        let mut label: Option<std::ffi::CString> = None;
        if emit {
            let collected = with_atom(|st| atoms_concat_keys(&st.vatom.atoms.borrow(), 0));
            let mut bytes = Vec::from(collected.as_bytes());
            if let Some(ref suf) = suffix {
                bytes.extend_from_slice(suf.as_bytes());
            }
            label = Some(bytes_to_cstring(&bytes));
        }
        atom_visual_reset();
        if emit {
            let mut staged_atom = CmdAtom::new();
            staged_atom.r#type = K_AVISUAL;
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
    let keys_cstr = bytes_to_cstring(&keys_bytes);
    let mut atom = CmdAtom::new();
    atom.r#type = K_AVISUAL;
    if let Some(spec) = spec {
        atom.spec = spec.without_body();
    }
    atom.keys = Some(keys_cstr);
    atom.origin = origin;
    if let Some(spec) = spec {
        let mut op_atom = CmdAtom::new();
        op_atom.r#type = K_AOPERATOR;
        op_atom.spec = spec.without_body();
        op_atom.keys = suffix;
        op_atom.origin = with_cur_frame(|f| f.origin);
        with_atom(|st| st.vatom.atoms.borrow_mut().push(op_atom));
    }
    let (cascaded, need_span) = with_atom(|st| {
        let done = st.vatom.done.get();
        let cascaded = done > 0;
        let need_span = cascaded && done < st.vatom.atoms.borrow().len();
        (cascaded, need_span)
    });
    atom.cascaded = cascaded;
    if need_span {
        let span_cstr = atom_visual_span();
        let mut span_atom = CmdAtom::new();
        span_atom.r#type = K_AVISUAL_SPAN;
        span_atom.keys = span_cstr;
        span_atom.origin = origin;
        with_atom(|st| st.g_atoms.borrow_mut().push(span_atom));
    }
    atom.atoms = with_atom(|st| std::mem::take(&mut *st.vatom.atoms.borrow_mut()));
    atom_visual_reset();
    atom_stage_set(&mut atom);
    prepped
}

pub fn atom_visual_end(spec: CmdSpec, redoable: bool) -> bool {
    if mc_replaying() {
        return false;
    }
    let suffix = atom_redo_keys(&spec);
    atom_visual_end_suffix(Some(suffix), Some(&spec), redoable)
}

pub fn atom_capture_op(oap: &mut Oparg, cap: &mut Cmdarg, redo_yank: bool) {
    let redoable = op_redoable(oap.op_type, redo_yank);
    let ins_op = oap.op_type == OP_CHANGE || oap.op_type == OP_INSERT || oap.op_type == OP_APPEND;
    let payload_motion =
        cap.cmdchar >= 0x100 || (cap.cmdchar != NUL && b"/?:!".contains(&(cap.cmdchar as u8)));
    let excmd = cap.cmdchar == b':' as i32 || cap.cmdchar == K_COMMAND;
    if !ins_op && !payload_motion {
        let prep_exempt = !redoable || cap.cmdchar == b'D' as i32;
        let mut spec = CmdSpec {
            regname: oap.regname,
            count: i64::from(cap.count0),
            op: get_op_char(oap.op_type),
            op_extra: get_extra_op_char(oap.op_type),
            ..CmdSpec::default()
        };
        if prep_exempt && (!g_st().visual_global.active || oap.motion_force != 0) {
            if atom_capturable(atom_buf_has_consumers(), g_st().key_typed) {
                let operand = nv_is(cap.cmdchar, NV_LANG | NV_NCH_ARG);
                spec.motion_force = oap.motion_force;
                spec.cmd = cap.cmdchar;
                spec.cmd2 = if operand { NUL } else { cap.nchar };
                spec.cmdarg = if operand { cap.nchar } else { NUL };
                let mut op_atom = atom_from_spec(K_AOPERATOR, spec);
                op_atom.origin = with_cur_frame(|f| f.origin);
                atom_stage_set(&mut op_atom);
            }
        } else if !g_st().visual_global.active || oap.motion_force != 0 {
            // Prepped: atom_cmd_end() derives the atom from redobuff.
        } else if oap.op_type == OP_REPLACE && cap.nchar <= 0 {
            let suffix: [u8; 3] = [
                b'r',
                CTRL_V_CONST as u8,
                if cap.nchar == REPLACE_CR_NCHAR {
                    CAR as u8
                } else {
                    NL as u8
                },
            ];
            atom_visual_end_suffix(Some(bytes_to_cstring(&suffix)), Some(&spec), redoable);
        } else {
            spec.cmdarg = if oap.op_type == OP_REPLACE {
                cap.nchar
            } else {
                NUL
            };
            if (oap.op_type == OP_NR_ADD || oap.op_type == OP_NR_SUB) && cap.arg != 0 {
                spec.op_extra = spec.op;
                spec.op = b'g' as i32;
            }
            atom_visual_end(spec, redoable);
        }
    }

    if redoable
        && cap.cmdchar != b'D' as i32
        && ((!g_st().visual_global.active || oap.motion_force != 0)
            || excmd
            || cap.cmdchar == K_LUA)
    {
        prep_redo(
            true,
            false,
            CmdSpec {
                regname: oap.regname,
                count: i64::from(cap.count0),
                op: get_op_char(oap.op_type),
                op_extra: get_extra_op_char(oap.op_type),
                motion_force: oap.motion_force,
                cmd: cap.cmdchar,
                cmd2: cap.nchar,
                ..CmdSpec::default()
            },
        );
        if cap.cmdchar == b'/' as i32 || cap.cmdchar == b'?' as i32 {
            if !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_REDO) {
                redo_append_lit(cap.searchbuf_cstr().map_or(&[][..], CStr::to_bytes));
            }
            redo_append_str(b"\n");
        } else if excmd {
            let rc = g_st().repeat_cmdline.take();
            if let Some(ref rc) = rc {
                if cap.cmdchar == b':' as i32 {
                    redo_append_lit(rc.as_bytes());
                } else {
                    redo_append_spec(rc.as_bytes());
                }
                redo_append_str(b"\n");
            } else {
                redo_new(CmdSpec::default());
            }
        } else if cap.cmdchar == K_LUA {
            let mut lua_id = [0u8; NUMBUFLEN];
            let n = format_to_buf!(&mut lua_id, c"%d", g_st().repeat_luamap);
            redo_append_str(&lua_id[..n.max(0) as usize]);
            redo_append_str(b"\n");
        }
    } else if g_st().visual_global.active && redoable && oap.motion_force == NUL {
        if op_self_select(cap) {
            prep_redo(
                true,
                false,
                CmdSpec {
                    regname: oap.regname,
                    count: i64::from(cap.count0),
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
            let v_bytes = if let Some(ref vc) = v {
                vc.as_bytes()
            } else {
                b"1v"
            };
            prep_redo_visual(
                v_bytes,
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

pub fn atom_ins_start(cmd: i32, count: i64, vis: VisualIns, vblock: bool) -> InsSession {
    let mut session = InsSession {
        typed: atom_is_user_input(),
        vis,
        origin: if vis == K_VINS_KEYS {
            with_atom(|st| st.vatom.origin.get())
        } else {
            with_cur_frame(|f| f.origin)
        },
        vsel: None,
    };

    if vis != K_VINS_NONE && !mc_replaying() && !is_child_frame() {
        if vis == K_VINS_OTHER && atom_visual_replayable() && !atom_visual_redoable() {
            session.vsel = atom_visual_span();
            session.origin = with_atom(|st| st.vatom.origin.get());
        }
        if vis == K_VINS_KEYS || session.vsel.is_some() {
            session.typed = atom_visual_replayable() && atom_visual_typed();
        }
        atom_visual_reset();
    }

    let repl = cmd == b'R' as i32 || cmd == b'V' as i32 || cmd == b'r' as i32 || cmd == b'v' as i32;
    let reexec = vis == K_VINS_NONE
        || vis == K_VINS_KEYS
        || session.vsel.is_some()
        || (vis == K_VINS_MOTION && with_cur_frame(|f| f.payload_start == usize::MAX));
    mc_ins_cascade_start(
        session.typed && count <= 1 && !repl && !vblock && reexec,
        session.origin,
        with_root_frame(|f| f.id),
        session.vsel.clone(),
    );

    session
}

pub fn atom_ins_end(session: &mut InsSession, busy: bool) {
    let visual = session.vis != K_VINS_NONE;
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
        || (visual && session.vis != K_VINS_KEYS && session.vsel.is_none())
    {
        if user_input && (busy || g_st().restart_edit != 0) && atom_composite_active() {
            with_atom(|st| st.composite.lossy.set(true));
        }
        session.vsel = None;
        return;
    }
    atom_ins_push(session, mc_buf_has_cursors(curbuf_mut()));
    session.vsel = None;
}

fn atom_ins_push(session: &InsSession, cascade: bool) {
    let mut atom = atom_from_redo(if session.vis != K_VINS_NONE {
        K_AVISUAL
    } else {
        K_AINSERT
    });
    if let Some(ref vsel) = session.vsel
        && let Some(ref cur_keys) = atom.keys
    {
        assert!(cur_keys.as_bytes().starts_with(b"1v"));
        let mut bytes = Vec::from(vsel.as_bytes());
        bytes.extend_from_slice(&cur_keys.as_bytes()[2..]);
        atom.keys = Some(bytes_to_cstring(&bytes));
    }
    let Some(ref cur_keys) = atom.keys else {
        return;
    };
    let key_bytes = cur_keys.as_bytes();
    let replace = atom.spec.cmd == b'r' as i32
        || (atom.spec.cmd == b'g' as i32 && atom.spec.cmd2 == b'r' as i32);
    if key_bytes.is_empty() || (!replace && key_bytes[key_bytes.len() - 1] != ESC) {
        return;
    }
    atom.text = get_last_insert_save();
    atom.origin = session.origin;
    atom_push_raw(cascade, &mut atom);
}

pub fn atom_cmd_start(cmdchar: i32) {
    let consumers = atom_buf_has_consumers();
    let origin = atom_origin();
    let follow = mc_following();
    let reg_ts = if consumers { reg_max_ts(true) } else { 0 };
    let keyclass = atom_key_class(cmdchar, NUL, NUL);
    let is_root = with_atom(|st| {
        let frame_id = st.frame_id.get() + 1;
        st.frame_id.set(frame_id);
        let is_root = st.cmd_frames.borrow().is_empty();
        st.cmd_frames.borrow_mut().push(CmdFrame {
            origin,
            staged: CmdAtom::default(),
            visual: g_st().visual_global,
            keytyped: g_st().key_typed,
            keyclass,
            ex_normal: g_st().ex_normal_busy,
            captures: st.atom_captures.get(),
            global_ops: st.global_ops.get(),
            beeps: g_st().did_beep,
            id: frame_id,
            follow,
            consumers,
            reg_ts,
            payload_start: usize::MAX,
            payload_end: 0,
            redo_frame: 0,
            cmdline: None,
        });
        if is_root && !st.composite.open.get() {
            st.composite.open.set(true);
            st.composite.origin.set(origin);
            st.composite.follow.set(K_NONE);
        }
        is_root
    });
    if is_root
        && !mc_replaying()
        && !atom_composite_active()
        && with_atom(|st| st.composite.lhs.get()[0] != 0)
    {
        if !g_st().key_typed && consumers {
            atom_composite_start(&[]);
            with_atom(|st| {
                st.typed.map_start.set(st.typed.keys.borrow().len());
                st.composite.follow.set(K_NONE);
            });
        } else {
            with_atom(|st| {
                let mut lhs = st.composite.lhs.get();
                lhs[0] = 0;
                st.composite.lhs.set(lhs);
            });
        }
    }
    if is_root && atom_is_user_cmd() && !atom_composite_active() {
        with_atom(|st| {
            st.typed.keys.borrow_mut().clear();
            st.typed.map_start.set(0);
        });
    }
}

fn atom_capture_cmd(ca: &mut Cmdarg, is_root: bool) -> bool {
    if mc_replaying() || with_atom(|st| st.atom_suppressed.get()) {
        return false;
    }

    let user = atom_is_user_cmd();
    let (
        root_id,
        root_redo_frame,
        root_has_cmdline,
        old_origin,
        old_visual,
        old_keytyped,
        old_captures,
        old_global_ops,
        old_beeps,
        old_id,
        old_consumers,
        old_reg_ts,
    ) = with_atom(|st| {
        let frames = st.cmd_frames.borrow();
        let root = &frames[0];
        let old = frames.last().unwrap();
        (
            root.id,
            root.redo_frame,
            root.cmdline.is_some(),
            old.origin,
            old.visual,
            old.keytyped,
            old.captures,
            old.global_ops,
            old.beeps,
            old.id,
            old.consumers,
            old.reg_ts,
        )
    });
    let keycls = atom_key_class(ca.cmdchar, ca.nchar, ca.extra_char);
    let opaque = (keycls & K_KEY_OPAQUE) != 0;
    let synthetic = (keycls & K_KEY_SYNTHETIC) != 0;
    let unchanged = old_origin.buf.br_buf == g_st().curbuf
        && old_origin.win == curwin_ptr()
        && !atom_origin_moved(old_origin)
        && !atom_origin_changed(old_origin)
        && g_st().visual_global.active == old_visual.active
        && (!g_st().visual_global.active
            || (equalpos(old_visual.start, g_st().visual_global.start)
                && old_visual.mode == g_st().visual_global.mode));
    if opaque
        && unchanged
        && !(atom_visual_pending() && with_atom(|st| st.frame_id.get() > old_id))
        && root_redo_frame != old_id
    {
        return false;
    }
    let ins_cascaded = user && mc_ins_cascaded(root_id);
    let mapped = user && !old_keytyped && !synthetic;
    if mapped
        && with_atom(|st| st.global_ops.get() == old_global_ops)
        && ((old_origin.buf.br_buf == g_st().curbuf && atom_origin_changed(old_origin))
            || ins_cascaded)
    {
        with_atom(|st| st.map_edit.set(true));
    }

    let child = is_child_frame();
    let mut vis = false;
    if g_st().visual_global.active && !child {
        if !old_visual.active && with_atom(|st| st.vatom.frame.get() <= old_id) {
            atom_visual_reset();
            with_atom(|st| {
                st.vatom.state.set(K_VATOM_FED);
                st.vatom.origin.set(old_origin);
            });
        }
        if atom_visual_pending() && user && !synthetic && (old_keytyped || atom_composite_active())
        {
            with_atom(|st| {
                st.vatom
                    .state
                    .set((st.vatom.state.get() & K_VATOM_VOID) | K_VATOM_TYPED);
            });
        }
        vis = atom_visual_pending() && ca.cmdchar != b'Q' as i32 && ca.cmdchar != b'q' as i32;
        if vis
            && (g_st().visual_global.select
                || ((keycls & (K_KEY_SCROLL_MOVE | K_KEY_SCROLL_VIEW | K_KEY_MOUSE)) != 0
                    && !unchanged))
        {
            with_atom(|st| st.vatom.state.set(st.vatom.state.get() | K_VATOM_VOID));
        }
    } else if old_visual.active && !child {
        let mut own = if atom_visual_pending() {
            atom_from_cur_frame(ca)
        } else {
            CmdAtom::default()
        };
        if let Some(suffix) = own.keys.take() {
            drop(own);
            atom_visual_end_suffix(Some(suffix), None, false);
        } else if user
            && atom_visual_replayable()
            && with_atom(|st| !st.vatom.atoms.borrow().is_empty())
        {
            atom_visual_end_suffix(Some(bytes_to_cstring(&[ESC])), None, false);
        } else {
            atom_visual_end_suffix(None, None, false);
        }
    }

    if (vis
        && with_atom(|st| st.vatom.frame.get() <= old_id && st.atom_captures.get() == old_captures)
        && ca.oap_mut().op_type == OP_NOP)
        || (!g_st().visual_global.active
            && !old_visual.active
            && atom_capturable(old_consumers, old_keytyped)
            && with_atom(|st| st.atom_captures.get() == old_captures)
            && !atom_staged()
            && ca.oap_mut().op_type == OP_NOP
            && stuff_empty()
            && !ins_cascaded)
    {
        let collected = with_atom(|st| st.vatom.atoms.borrow().len());
        let special_motion = (keycls & K_KEY_MOTION) != 0;
        let scroll_cmd = (keycls & (K_KEY_SCROLL_MOVE | K_KEY_SCROLL_VIEW)) != 0;
        let mouse_cmd = (keycls & K_KEY_MOUSE) != 0;
        let jump_cmd = nv_is(ca.cmdchar, NV_JUMP)
            || (keycls & K_KEY_JUMP) != 0
            || (ca.cmdchar == b'%' as i32 && ca.count0 > 0);
        let replayable = (ca.cmdchar > 0
            && ca.cmdchar < 0x100
            && ca.cmdchar != b'"' as i32
            && ca.cmdchar != b'@' as i32
            && !scroll_cmd)
            || special_motion;
        let changed = atom_origin_changed(old_origin);
        let motion = (nv_is(ca.cmdchar, NV_MOTION) || special_motion)
            && !changed
            && !g_st().finish_op
            && !jump_cmd;
        let failed = g_st().did_beep != old_beeps && !atom_origin_moved(old_origin);
        let follow = (mc_following() || mapped) && motion && !failed;

        if root_redo_frame == old_id && !mouse_cmd {
            let mut atom = atom_from_redo(K_AOPERATOR);
            atom_payload_append_cur_frame(&mut atom);

            let effect = (changed
                || reg_max_ts(true) > old_reg_ts
                || (mc_following() && atom_origin_moved(old_origin)))
                && with_atom(|st| st.global_ops.get() == old_global_ops);
            if atom.keys.as_ref().is_some_and(|k| !k.as_bytes().is_empty()) {
                atom.origin = old_origin;
                atom_push(effect, &mut atom);
            }
        } else if ca.searchbuf.is_some()
            && (ca.cmdchar == b'/' as i32 || ca.cmdchar == b'?' as i32)
            && !(vis && unchanged)
        {
            let mut atom = atom_from_cmdline(K_AMOTION, ca, ca.searchbuf_cstr(), old_visual.active);
            atom.origin = old_origin;
            atom_push(false, &mut atom);
        } else if (root_has_cmdline && (ca.cmdchar == b':' as i32 || ca.cmdchar == K_COMMAND))
            || (vis && ca.cmdchar == K_LUA)
        {
            let mut atom = atom_from_cur_frame(ca);
            atom.origin = old_origin;
            atom_push(false, &mut atom);
        } else if replayable && (!vis || ((keycls & K_KEY_PAYLOAD) == 0 && !failed)) {
            let mut spec = atom_cmd_spec(ca);
            if vis {
                spec.regname = 0;
            }
            let mut atom = atom_from_spec(
                if motion {
                    K_AMOTION
                } else if jump_cmd {
                    K_AJUMP
                } else {
                    K_ANORMAL
                },
                spec,
            );
            atom.origin = old_origin;
            atom_push(follow, &mut atom);
            if jump_cmd && atom_composite_active() {
                with_atom(|st| st.composite.queued.set(true));
            }
        } else if (scroll_cmd || mouse_cmd) && !atom_composite_active() {
            let mut spec = atom_cmd_spec(ca);
            if is_special(ca.cmdchar) {
                spec.count = 0;
            }
            let mut atom = atom_from_spec(if scroll_cmd { K_ASCROLL } else { K_AMOUSE }, spec);
            atom.origin = old_origin;
            atom_push(false, &mut atom);
        }
        if vis && with_atom(|st| st.vatom.atoms.borrow().len() == collected) && !unchanged {
            with_atom(|st| st.vatom.state.set(st.vatom.state.get() | K_VATOM_VOID));
        }
    }

    if vis
        && (old_origin.buf.br_buf != g_st().curbuf
            || (atom_origin_changed(old_origin) && with_atom(|st| st.vatom.frame.get() != old_id)))
    {
        with_atom(|st| st.vatom.state.set(st.vatom.state.get() | K_VATOM_VOID));
    }
    atom_visual_span_flush();

    g_st().visual_global.active && user && is_root
}

pub fn atom_cmd_end(ca: &mut Cmdarg) {
    let (is_root, old_origin, old_follow) = with_atom(|st| {
        let frames = st.cmd_frames.borrow();
        let old = frames.last().unwrap();
        (frames.len() == 1, old.origin, old.follow)
    });
    if with_atom(|st| st.composite.follow.get() == K_NONE)
        && !mc_replaying()
        && (atom_origin_moved(old_origin) || nv_is(ca.cmdchar, NV_MOTION))
    {
        with_atom(|st| {
            st.composite
                .follow
                .set(if old_follow { K_TRUE } else { K_FALSE });
        });
    }
    let refresh_visual = atom_capture_cmd(ca, is_root);
    atom_stage_flush();
    let (old_payload_start, old_payload_end) =
        with_cur_frame(|old| (old.payload_start, old.payload_end));
    if atom_composite_active()
        && old_payload_start != usize::MAX
        && old_payload_end > old_payload_start
    {
        with_atom(|st| st.composite.lossy.set(true));
    }

    if is_root && !mc_replaying() && typebuf_typed() != 0 && stuff_empty() {
        let (map_edit, comp_origin, comp_follow) = with_atom(|st| {
            (
                st.map_edit.get(),
                st.composite.origin.get(),
                st.composite.follow.get() == K_TRUE,
            )
        });
        let map_moved = atom_composite_active() && atom_origin_moved(comp_origin);
        mc_clock_edge(map_edit, map_moved, comp_follow);

        with_atom(|st| st.map_edit.set(false));
        if ca.oap_mut().op_type == OP_NOP
            && !g_st().visual_global.active
            && g_st().restart_edit == 0
        {
            atom_composite_end();
            with_atom(|st| st.composite.open.set(false));
        }
    }
    if refresh_visual {
        mc_vsel_refresh();
    }
    with_atom(|st| {
        st.cmd_frames.borrow_mut().pop();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sb_concat_works() {
        let mut sb: Vec<u8> = Vec::new();
        sb_concat(&mut sb, b"abc");
        sb_concat(&mut sb, b"def");
        assert_eq!(sb, b"abcdef");

        let mut v = CmdAtomVec::new();
        v.clear();
        assert!(v.is_empty());
    }
}
