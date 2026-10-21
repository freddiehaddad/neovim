//! Input engine command-atom policy layer (`src/nvim/input_cmdatom.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments,
    clippy::useless_conversion
)]

use crate::api::private::helpers::{
    api_free_dict, api_free_string, array_obj, boolean_obj, cstr_as_obj, cstr_to_obj,
    cstr_to_string, dict_obj, integer_obj,
};
use crate::api::private::validate::{Array, Dict, KeyValuePair, NvimString, Object};
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
use crate::kvec::{KVec, StringBuilder};
use crate::mbyte::utf_char2bytes;
use crate::mcursor::{
    mc_buf_has_cursors, mc_clock_edge, mc_following, mc_ins_cascade_start, mc_ins_cascaded,
    mc_ins_commit, mc_mark_at, mc_replaying, mc_vsel_clear, mc_vsel_refresh,
};
use crate::memory::{xfree, xmemdupz, xrealloc, xstrdup};
use crate::normal::{REPLACE_CR_NCHAR, VisualState, nv_is};
use crate::ops::{
    OP_APPEND, OP_CHANGE, OP_INSERT, OP_NOP, OP_NR_ADD, OP_NR_SUB, OP_REPLACE, get_extra_op_char,
    get_op_char, op_redoable, op_self_select,
};
use crate::option_vars::*;
use crate::register::reg_max_ts;
use crate::state::{MODE_NORMAL, MODE_OP_PENDING, get_real_state};
use crate::strings::{
    concat_str, strcat, strchr, strcpy, strlen, strncmp, vim_snprintf, vim_strchr,
};
use crate::textformat::oparg_T;
use crate::viml::parser::expressions::ESC;
use crate::window::{win_T, win_valid};
use std::ffi::{CStr, c_char, c_int, c_long, c_uint};
use std::ptr;

pub type CmdAtomVec = KVec<CmdAtom>;
use crate::os::time::Timestamp;

use crate::api::ui::MAX_SCHAR_SIZE;
use crate::cmdexpand::MB_MAXBYTES;
use crate::ex_cmds::{Ctrl_E, Ctrl_Y};
use crate::ex_getln::{
    Ctrl_B, Ctrl_D, Ctrl_F, Ctrl_G, Ctrl_R, Ctrl_T, Ctrl_U, Ctrl_W, K_EVENT, K_LUA,
};
use crate::mapping::MAXMAPLEN;
use crate::regexp::{Ctrl_H, Ctrl_V, NUMBUFLEN};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct cmdarg_T {
    pub oap: *mut oparg_T,
    pub prechar: c_int,
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
    pub searchbuf: *mut c_char,
}

const NV_LANG: c_int = 0x08;
const NV_NCH_ARG: c_int = 0x400;
const NV_MOTION: c_int = 0x800;
const NV_JUMP: c_int = 0x1000;

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
#[derive(Clone, Copy, Default)]
pub struct CmdSpec {
    pub count: c_long,
    pub regname: c_int,
    pub op: c_int,
    pub op_extra: c_int,
    pub motion_force: c_int,
    pub cmd: c_int,
    pub cmd2: c_int,
    pub cmdarg: c_int,
    pub body: StringBuilder,
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

impl Default for CmdOrigin {
    fn default() -> Self {
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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InsSession {
    pub origin: CmdOrigin,
    pub typed: bool,
    pub vis: VisualIns,
    pub vsel: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CmdAtom {
    pub r#type: CmdAtomType,
    pub spec: CmdSpec,
    pub origin: CmdOrigin,
    pub atoms: CmdAtomVec,
    pub keys: *mut c_char,
    pub text: *mut c_char,
    pub lhs: *mut c_char,
    pub undoseq: c_int,
    pub changed: bool,
    pub moved: bool,
    pub remap: bool,
    pub cascaded: bool,
}

impl Default for CmdAtom {
    fn default() -> Self {
        Self {
            r#type: kAComp,
            spec: CmdSpec::default(),
            origin: CmdOrigin::default(),
            atoms: KVec::new(),
            keys: ptr::null_mut(),
            text: ptr::null_mut(),
            lhs: ptr::null_mut(),
            undoseq: 0,
            changed: false,
            moved: false,
            remap: false,
            cascaded: false,
        }
    }
}

#[repr(C)]
pub struct CmdFrame {
    pub origin: CmdOrigin,
    pub staged: CmdAtom,
    pub parent: *mut CmdFrame,
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
    pub cmdline: *mut c_char,
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
    keys: KVec<u8>,
    map_start: usize,
}

#[unsafe(no_mangle)]
pub static mut g_atoms: CmdAtomVec = KVec::new();

static mut atom_captures: u64 = 0;
static mut global_ops: u64 = 0;
static mut atom_suppressed: bool = false;
static mut map_edit: bool = false;
static mut frame_id: u64 = 0;
static mut cur_frame: *mut CmdFrame = ptr::null_mut();

static mut composite: CompositeState = CompositeState {
    active: false,
    open: false,
    origin: CmdOrigin {
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
    },
    follow: kNone,
    atoms: KVec::new(),
    lhs: [0; MAXMAPLEN + 4],
    queued: false,
    lossy: false,
    r#macro: false,
    frame: 0,
};

static mut vatom: VatomStateStruct = VatomStateStruct {
    atoms: KVec::new(),
    state: kVatomNone,
    origin: CmdOrigin {
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
    },
    frame: 0,
    done: 0,
};

static mut typed: TypedState = TypedState {
    keys: KVec::new(),
    map_start: 0,
};

#[inline]
pub unsafe fn sb_concat(sb: &mut StringBuilder, s: *const c_char) {
    unsafe {
        let len = strlen(s);
        sb.concat_len(s, len);
    }
}

#[inline]
unsafe fn dict_put(d: &mut Dict, key: &CStr, value: Object) {
    unsafe {
        let mut kv: KVec<KeyValuePair> = KVec {
            size: d.size,
            capacity: d.capacity,
            items: d.items,
        };
        kv.push(KeyValuePair {
            key: cstr_to_string(key.as_ptr()),
            value,
        });
        d.size = kv.size;
        d.capacity = kv.capacity;
        d.items = kv.items;
    }
}

#[inline]
unsafe fn array_add(a: &mut Array, item: Object) {
    unsafe {
        let mut kv: KVec<Object> = KVec {
            size: a.size,
            capacity: a.capacity,
            items: a.items,
        };
        kv.push(item);
        a.size = kv.size;
        a.capacity = kv.capacity;
        a.items = kv.items;
    }
}

#[inline]
unsafe fn root_frame() -> *mut CmdFrame {
    unsafe {
        assert!(!cur_frame.is_null());
        let mut frame = cur_frame;
        while !(*frame).parent.is_null() {
            frame = (*frame).parent;
        }
        frame
    }
}

#[inline]
unsafe fn is_child_frame() -> bool {
    unsafe {
        if cur_frame.is_null() || (*cur_frame).parent.is_null() {
            return false;
        }
        let root = root_frame();
        let fed = (*cur_frame).ex_normal > (*root).ex_normal;
        fed && ((*root).keyclass & kKeySynthetic) == 0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_free(atom: *mut CmdAtom) {
    unsafe {
        if !(*atom).keys.is_null() {
            xfree((*atom).keys.cast());
            (*atom).keys = ptr::null_mut();
        }
        if !(*atom).text.is_null() {
            xfree((*atom).text.cast());
            (*atom).text = ptr::null_mut();
        }
        if !(*atom).lhs.is_null() {
            xfree((*atom).lhs.cast());
            (*atom).lhs = ptr::null_mut();
        }
        atoms_free(&mut (*atom).atoms);
        (*atom).atoms.destroy();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atoms_free(v: *mut CmdAtomVec) {
    unsafe {
        while (*v).size > 0 {
            (*v).size -= 1;
            let mut atom = *(*v).items.add((*v).size);
            atom_free(&mut atom);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_free_all() {
    unsafe {
        atoms_free(&raw mut g_atoms);
        crate::kvec::kv_destroy(&raw mut g_atoms);
        let mut frame = cur_frame;
        while !frame.is_null() {
            atom_free(&mut (*frame).staged);
            if !(*frame).cmdline.is_null() {
                xfree((*frame).cmdline.cast());
                (*frame).cmdline = ptr::null_mut();
            }
            frame = (*frame).parent;
        }
        atom_composite_abort();
        crate::kvec::kv_destroy(&raw mut composite.atoms);
        crate::kvec::kv_destroy(&raw mut typed.keys);
        atoms_free(&raw mut vatom.atoms);
        crate::kvec::kv_destroy(&raw mut vatom.atoms);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_cmd_spec(cap: *const cmdarg_T) -> CmdSpec {
    unsafe {
        let operand = nv_is((*cap).cmdchar, NV_LANG | NV_NCH_ARG);
        CmdSpec {
            regname: (*(*cap).oap).regname,
            count: c_long::from((*cap).count0),
            cmd: (*cap).cmdchar,
            cmd2: if operand { NUL } else { (*cap).nchar },
            cmdarg: if operand {
                (*cap).nchar
            } else {
                (*cap).extra_char
            },
            ..CmdSpec::default()
        }
    }
}

unsafe fn atom_origin() -> CmdOrigin {
    unsafe {
        let mut origin = CmdOrigin {
            win: curwin,
            pos: (*curwin).w_cursor,
            tick: buf_get_changedtick(curbuf),
            maptick,
            mcursor: mc_mark_at(curbuf, (*curwin).w_cursor, 0),
            ..CmdOrigin::default()
        };
        set_bufref(&mut origin.buf, curbuf);
        origin
    }
}

#[inline]
unsafe fn atom_origin_changed(origin: CmdOrigin) -> bool {
    unsafe {
        bufref_valid((&raw const origin.buf).cast_mut())
            && buf_get_changedtick(origin.buf.br_buf) != origin.tick
    }
}

#[inline]
unsafe fn atom_origin_moved(origin: CmdOrigin) -> bool {
    unsafe {
        bufref_valid((&raw const origin.buf).cast_mut())
            && win_valid(origin.win)
            && (*origin.win).w_buffer == origin.buf.br_buf
            && !equalpos(origin.pos, (*origin.win).w_cursor)
    }
}

#[inline]
unsafe fn atom_origin_undoseq(origin: CmdOrigin) -> c_int {
    unsafe {
        if bufref_valid((&raw const origin.buf).cast_mut()) {
            (*origin.buf.br_buf).b_u_seq_cur
        } else {
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_origin_pos(buf: *mut buf_T) -> pos_T {
    unsafe {
        let o = composite.origin;
        if composite.open
            && o.buf.br_buf == buf
            && bufref_valid((&raw const o.buf).cast_mut())
            && buf_get_changedtick(buf) == o.tick
        {
            return o.pos;
        }
        (*curwin).w_cursor
    }
}

unsafe fn atom_redo_keys(spec: CmdSpec) -> *mut c_char {
    unsafe {
        assert!(!mc_replaying());
        let keys = redo_keys(&spec).data;
        assert!(!keys.is_null());
        keys
    }
}

unsafe fn atom_from_redo(r#type: CmdAtomType) -> CmdAtom {
    unsafe {
        let keys = redo_keys(ptr::null());
        CmdAtom {
            r#type,
            spec: redo_spec(),
            keys: keys.data,
            ..CmdAtom::default()
        }
    }
}

unsafe fn atom_from_spec(r#type: CmdAtomType, spec: CmdSpec) -> CmdAtom {
    unsafe {
        CmdAtom {
            r#type,
            spec,
            keys: atom_redo_keys(spec),
            ..CmdAtom::default()
        }
    }
}

unsafe fn atom_from_cmdline(
    r#type: CmdAtomType,
    ca: *mut cmdarg_T,
    cmdline: *const c_char,
    vis: bool,
) -> CmdAtom {
    unsafe {
        let mut sb: StringBuilder = KVec::new();
        if (*ca).cmdchar != b':' as c_int && (*ca).count0 != 0 {
            let mut num_buf = [0 as c_char; 32];
            let n = vim_snprintf!(
                num_buf.as_mut_ptr(),
                num_buf.len(),
                c"%d".as_ptr(),
                (*ca).count0,
            );
            if n > 0 {
                sb.concat_len(num_buf.as_ptr(), n as usize);
            }
        }
        sb_add_char(&mut sb, (*ca).cmdchar);
        if vis && (*ca).cmdchar == b':' as c_int {
            sb_add_char(&mut sb, Ctrl_U);
        }
        if is_special((*ca).cmdchar) {
            sb_add_spec(&mut sb, cmdline);
        } else {
            sb_add_lit(&mut sb, cmdline, -1);
        }
        sb_add_char(&mut sb, NL);
        sb.push(0);
        CmdAtom {
            r#type,
            spec: CmdSpec {
                count: c_long::from((*ca).count0),
                cmd: (*ca).cmdchar,
                ..CmdSpec::default()
            },
            keys: sb.items,
            text: xstrdup(cmdline),
            ..CmdAtom::default()
        }
    }
}

unsafe fn atom_from_frame(ca: *mut cmdarg_T, frame: *mut CmdFrame) -> CmdAtom {
    unsafe {
        let root = root_frame();
        let mut atom = CmdAtom::default();
        if !(*root).cmdline.is_null()
            && ((*ca).cmdchar == b':' as c_int || (*ca).cmdchar == K_COMMAND)
        {
            atom = atom_from_cmdline(kAExcmd, ca, (*root).cmdline, (*frame).visual.active);
        } else if (*ca).cmdchar == K_LUA {
            let mut lua_id = [0 as c_char; NUMBUFLEN];
            vim_snprintf!(
                lua_id.as_mut_ptr(),
                lua_id.len(),
                c"%d".as_ptr(),
                repeat_luamap,
            );
            atom = atom_from_cmdline(kANormal, ca, lua_id.as_ptr(), (*frame).visual.active);
        }
        atom_payload_append(&mut atom, frame);
        atom
    }
}

unsafe fn atoms_concat_keys(atoms: CmdAtomVec, from: usize) -> NvimString {
    unsafe {
        let mut keys: StringBuilder = KVec::new();
        for i in from..atoms.size {
            let sub_keys = (*atoms.items.add(i)).keys;
            sb_concat(&mut keys, sub_keys);
        }
        let len = keys.size;
        keys.push(0);
        NvimString {
            data: keys.items,
            size: len,
        }
    }
}

unsafe fn atom_key_name(c: c_int) -> *mut c_char {
    unsafe {
        if c == NUL || c == K_LUA {
            return xstrdup(c"".as_ptr());
        }
        if is_special(c) || c < b' ' as c_int {
            return xstrdup(get_special_key_name(c, 0));
        }
        if c == DEL {
            return xstrdup(c"<Del>".as_ptr());
        }
        let mut buf = [0 as c_char; MB_MAXBYTES + 1];
        let n = utf_char2bytes(c, buf.as_mut_ptr()) as usize;
        buf[n] = 0;
        xstrdup(buf.as_ptr())
    }
}

unsafe fn atom_dict(atom: *const CmdAtom) -> Dict {
    unsafe {
        let spec = &(*atom).spec;
        let regname: [c_char; 2] = [spec.regname as c_char, 0];
        let force_buf: [c_char; 2] = [spec.motion_force as c_char, 0];
        let force: *const c_char = if spec.motion_force == Ctrl_V {
            c"<C-V>".as_ptr()
        } else {
            force_buf.as_ptr()
        };

        let mut op = atom_key_name(spec.op);
        if spec.op_extra != NUL {
            let extra = atom_key_name(spec.op_extra);
            op = xrealloc(op.cast(), strlen(op) + strlen(extra) + 1).cast();
            strcat(op, extra);
            xfree(extra.cast());
        }
        let mut cmd = atom_key_name(spec.cmd);
        if spec.cmd2 != NUL {
            let cmd2 = atom_key_name(spec.cmd2);
            cmd = xrealloc(cmd.cast(), strlen(cmd) + strlen(cmd2) + 1).cast();
            strcat(cmd, cmd2);
            xfree(cmd2.cast());
        }

        let mut d = Dict {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        let cmdarg = atom_key_name(spec.cmdarg);
        if !cmdarg.is_null() && *cmdarg != 0 {
            dict_put(&mut d, c"cmdarg", cstr_as_obj(cmdarg));
        } else {
            xfree(cmdarg.cast());
        }
        dict_put(&mut d, c"changed", boolean_obj((*atom).changed));
        if *cmd != 0 {
            dict_put(&mut d, c"cmd", cstr_as_obj(cmd));
        } else {
            xfree(cmd.cast());
        }
        if spec.count > 0 {
            dict_put(&mut d, c"count", integer_obj(i64::from(spec.count)));
        }
        if (*atom).origin.pos.lnum > 0 {
            dict_put(&mut d, c"moved", boolean_obj((*atom).moved));
            let mut pos = Array {
                items: ptr::null_mut(),
                size: 0,
                capacity: 0,
            };
            array_add(&mut pos, integer_obj(i64::from((*atom).origin.pos.lnum)));
            array_add(&mut pos, integer_obj(i64::from((*atom).origin.pos.col)));
            dict_put(&mut d, c"pos", array_obj(pos));
            dict_put(&mut d, c"undoseq", integer_obj(i64::from((*atom).undoseq)));
        }
        let keys: *const c_char = if !(*atom).keys.is_null() {
            (*atom).keys
        } else {
            c"".as_ptr()
        };
        if !(*atom).remap {
            dict_put(&mut d, c"keys", cstr_to_obj(keys));
        }
        let lhs_ptr = if !(*atom).lhs.is_null() && *(*atom).lhs != 0 {
            (*atom).lhs.cast_const()
        } else {
            keys
        };
        dict_put(&mut d, c"lhs", cstr_to_obj(lhs_ptr));
        if *force != 0 {
            dict_put(&mut d, c"motionforce", cstr_to_obj(force));
        }
        if *op != 0 {
            dict_put(&mut d, c"operator", cstr_as_obj(op));
        } else {
            xfree(op.cast());
        }
        if spec.regname != 0 {
            dict_put(&mut d, c"reg", cstr_to_obj(regname.as_ptr()));
        }
        if !(*atom).text.is_null() && *(*atom).text != 0 {
            dict_put(&mut d, c"text", cstr_to_obj((*atom).text));
        }
        dict_put(
            &mut d,
            c"type",
            cstr_to_obj(TYPE_NAMES[(*atom).r#type as usize].as_ptr()),
        );
        d
    }
}

unsafe fn atom_emit(atom: *const CmdAtom) {
    unsafe {
        if !has_event(EVENT_CMDATOM) {
            return;
        }
        let mut data = atom_dict(atom);
        if (*atom).atoms.size > 0 {
            let mut atoms = Array {
                items: ptr::null_mut(),
                size: 0,
                capacity: 0,
            };
            for i in 0..(*atom).atoms.size {
                array_add(&mut atoms, dict_obj(atom_dict((*atom).atoms.items.add(i))));
            }
            dict_put(&mut data, c"atoms", array_obj(atoms));
        }
        let buf = if !(*atom).origin.buf.br_buf.is_null() {
            (*atom).origin.buf.br_buf
        } else {
            curbuf
        };
        let mut obj = dict_obj(data);
        aucmd_defer(
            EVENT_CMDATOM,
            TYPE_NAMES[(*atom).r#type as usize].as_ptr().cast_mut(),
            ptr::null_mut(),
            AUGROUP_ALL,
            buf,
            ptr::null_mut(),
            &mut obj,
        );
        api_free_dict(data);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_push_raw(cascade: bool, atom: *mut CmdAtom) {
    unsafe {
        assert!(!(*atom).keys.is_null());
        if !(*atom).origin.buf.br_buf.is_null() {
            (*atom).changed = atom_origin_changed((*atom).origin);
            (*atom).moved = atom_origin_moved((*atom).origin);
            (*atom).undoseq = atom_origin_undoseq((*atom).origin);
        }
        let collect =
            composite.active && (cur_frame.is_null() || (*cur_frame).id != composite.frame);
        if atom_visual_pending() {
            if (vatom.state & kVatomTyped) != 0 {
                atom_captures += 1;
            }
            if collect {
                composite.queued = true;
            }
            crate::kvec::kv_push(&raw mut vatom.atoms, *atom);
            vatom.frame = if !cur_frame.is_null() {
                (*cur_frame).id
            } else {
                0
            };
            return;
        }
        atom_captures += 1;
        if (*atom).r#type == kAVisual && (*atom).atoms.size > 0 {
            let last = (*atom).atoms.items.add((*atom).atoms.size - 1);
            if (*last).r#type == kAOperator {
                (*last).changed = (*atom).changed;
            }
        }
        if cascade {
            let mut copy = *atom;
            copy.keys = xstrdup((*atom).keys);
            copy.text = ptr::null_mut();
            copy.lhs = ptr::null_mut();
            copy.atoms = KVec::new();
            crate::kvec::kv_push(&raw mut g_atoms, copy);
            if collect {
                composite.queued = true;
            }
        }
        if collect {
            crate::kvec::kv_push(&raw mut composite.atoms, *atom);
        } else {
            if (*atom).r#type != kAInsertSpan {
                atom_emit(atom);
            }
            atom_free(atom);
        }
    }
}

unsafe fn atom_push(cascade: bool, atom: *mut CmdAtom) {
    unsafe {
        if atom_blocked() {
            atom_free(atom);
            return;
        }
        atom_push_raw(cascade && mc_buf_has_cursors(curbuf), atom);
    }
}

unsafe fn atom_stage_set(atom: *mut CmdAtom) {
    unsafe {
        assert!(!cur_frame.is_null());
        assert!(!atom_staged());
        atom_free(&mut (*cur_frame).staged);
        if atom_blocked() {
            atom_free(atom);
            return;
        }
        assert!(!(*atom).keys.is_null());
        (*cur_frame).staged = *atom;
    }
}

#[inline]
unsafe fn atom_staged() -> bool {
    unsafe { !cur_frame.is_null() && !(*cur_frame).staged.keys.is_null() }
}

unsafe fn atom_stage_flush(frame: *mut CmdFrame) {
    unsafe {
        if (*frame).staged.keys.is_null() {
            return;
        }
        let cascade = *(*frame).staged.keys != 0
            && !(*frame).staged.cascaded
            && global_ops == (*frame).global_ops;
        atom_push(cascade, &mut (*frame).staged);
        (*frame).staged = CmdAtom::default();
    }
}

unsafe fn atom_composite_lhs() -> *mut c_char {
    unsafe {
        let mut keys: StringBuilder = KVec::new();
        sb_concat(&mut keys, ptr::addr_of!(composite.lhs).cast());
        keys.concat_len(
            typed.keys.items.add(typed.map_start).cast(),
            typed.keys.size - typed.map_start,
        );
        keys.push(0);
        keys.items
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_lhs_replay_queue() {
    unsafe {
        crate::kvec::kv_push(
            &raw mut g_atoms,
            CmdAtom {
                r#type: kAComp,
                keys: atom_composite_lhs(),
                remap: true,
                origin: composite.origin,
                ..CmdAtom::default()
            },
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_composite_queued() -> bool {
    unsafe { composite.queued }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_composite_active() -> bool {
    unsafe { composite.active }
}

unsafe fn atom_composite_label(lhs: *const c_char, len: usize) {
    unsafe {
        let copy_len = len.min(MAXMAPLEN + 3);
        ptr::copy_nonoverlapping(lhs, ptr::addr_of_mut!(composite.lhs).cast(), copy_len);
        composite.lhs[copy_len] = 0;
    }
}

unsafe fn atom_composite_start(lhs: *const c_char, len: usize) {
    unsafe {
        if !lhs.is_null() {
            atom_composite_label(lhs, len);
        }
        composite.active = true;
        composite.queued = false;
        composite.lossy = false;
        composite.frame = 0;
    }
}

unsafe fn atom_composite_end() {
    unsafe {
        composite.r#macro = false;
        if !composite.active {
            return;
        }
        let remap = composite.lossy || composite.atoms.size == 0;
        let lhs = atom_composite_lhs();
        composite.lhs[0] = 0;
        composite.active = false;
        let mut atom: CmdAtom;
        if composite.atoms.size == 1 {
            composite.atoms.size -= 1;
            atom = *composite.atoms.items.add(composite.atoms.size);
            xfree(atom.lhs.cast());
            atom.lhs = lhs;
            atom.remap = remap;
        } else {
            atom = CmdAtom {
                r#type: kAComp,
                keys: atoms_concat_keys(composite.atoms, 0).data,
                lhs,
                remap,
                origin: composite.origin,
                changed: atom_origin_changed(composite.origin),
                moved: atom_origin_moved(composite.origin),
                undoseq: atom_origin_undoseq(composite.origin),
                atoms: composite.atoms,
                ..CmdAtom::default()
            };
            composite.atoms = KVec::new();
        }
        atom_emit(&atom);
        atom_free(&mut atom);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_composite_abort() {
    unsafe {
        composite.r#macro = false;
        composite.lhs[0] = 0;
        composite.active = false;
        atoms_free(&raw mut composite.atoms);
        composite.open = false;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_term_enter() {
    unsafe {
        if !mc_replaying() {
            atom_composite_end();
            composite.open = false;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_is_user_cmd() -> bool {
    unsafe {
        !mc_replaying()
            && ((reg_executing == 0 && !pending_end_reg_executing) || composite.r#macro)
            && ex_normal_busy == 0
    }
}

#[inline]
unsafe fn atom_is_user_input() -> bool {
    unsafe { KeyTyped || (atom_is_user_cmd() && atom_composite_active()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_suppress(suppress: bool) {
    unsafe {
        atom_suppressed = suppress;
    }
}

#[inline]
unsafe fn atom_blocked() -> bool {
    unsafe { mc_replaying() || atom_suppressed }
}

#[inline]
unsafe fn atom_capturable(consumers: bool, keytyped: bool) -> bool {
    unsafe { consumers && atom_is_user_cmd() && (keytyped || atom_composite_active()) }
}

#[inline]
unsafe fn atom_buf_has_consumers() -> bool {
    unsafe { mc_buf_has_cursors(curbuf) || has_event(EVENT_CMDATOM) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_key_class(cmd: c_int, arg: c_int, arg2: c_int) -> c_uint {
    unsafe {
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
            if !strchr(c";,go".as_ptr(), arg).is_null() {
                return kKeyJump;
            }
            return if !strchr(c"jk0^$_meEM*#".as_ptr(), arg).is_null() {
                kKeyMotion
            } else {
                0
            };
        }
        if cmd == b'[' as c_int || cmd == b']' as c_int {
            if arg == b'C' as c_int {
                return kKeyJump;
            }
            return if !strchr(c"[](){}mMcsz#*/".as_ptr(), arg).is_null() {
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
            return if arg != NUL && !strchr(c"<>[].^(){}".as_ptr(), arg).is_null() {
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
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_cmdline_set(firstc: c_int, line: *const c_char, len: usize) {
    unsafe {
        if cur_frame.is_null()
            || !atom_is_user_cmd()
            || (firstc != b':' as c_int && firstc != K_COMMAND)
        {
            return;
        }
        let root = root_frame();
        if (*root).cmdline.is_null() {
            (*root).cmdline = xmemdupz(line.cast(), len).cast();
        }
    }
}

#[inline]
unsafe fn atom_typed_collecting() -> bool {
    unsafe {
        atom_composite_active()
            || (!cur_frame.is_null() && (*cur_frame).payload_start != usize::MAX)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_payload_start() {
    unsafe {
        if expr_map_lock == 0
            && !cur_frame.is_null()
            && (*cur_frame).payload_start == usize::MAX
            && !mc_replaying()
        {
            (*cur_frame).payload_start = typed.keys.size;
            (*cur_frame).payload_end = typed.keys.size;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_payload_end() {
    unsafe {
        if expr_map_lock == 0
            && !cur_frame.is_null()
            && (*cur_frame).payload_start != usize::MAX
            && !mc_replaying()
        {
            (*cur_frame).payload_end = typed.keys.size;
        }
    }
}

unsafe fn atom_payload_append(atom: *mut CmdAtom, frame: *mut CmdFrame) {
    unsafe {
        let plen = if (*frame).payload_start == usize::MAX {
            0
        } else {
            (*frame).payload_end - (*frame).payload_start
        };
        if plen == 0 || (*atom).keys.is_null() {
            return;
        }
        let klen = strlen((*atom).keys);
        (*atom).keys = xrealloc((*atom).keys.cast(), klen + plen + 1).cast();
        ptr::copy_nonoverlapping(
            typed.keys.items.add((*frame).payload_start),
            (*atom).keys.add(klen).cast(),
            plen,
        );
        *(*atom).keys.add(klen + plen) = 0;
        (*frame).payload_start = usize::MAX;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_typed_add(chars: *const u8, len: usize) {
    unsafe {
        if mc_replaying() || !atom_typed_collecting() || cmdline_star > 0 {
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
        for i in 0..len {
            crate::kvec::kv_push(&raw mut typed.keys, *chars.add(i));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_typed_del(len: usize) {
    unsafe {
        if mc_replaying() || !atom_typed_collecting() {
            return;
        }
        typed.keys.size -= len.min(typed.keys.size);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_did_global_op() {
    unsafe {
        if !mc_replaying() {
            global_ops += 1;
            composite.queued = true;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_follow_changed() {
    unsafe {
        if mc_replaying() {
            return;
        }
        let mut frame = cur_frame;
        while !frame.is_null() {
            if !atom_origin_moved((*frame).origin) {
                (*frame).follow = mc_following();
            } else if composite.follow == kNone {
                composite.follow = if (*frame).follow { kTrue } else { kFalse };
            }
            frame = (*frame).parent;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_redo_prepped() {
    unsafe {
        if atom_is_user_cmd() {
            (*root_frame()).redo_frame = (*cur_frame).id;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_redo_cancel() {
    unsafe {
        (*root_frame()).redo_frame = 0;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_macro_start(regname: c_int) {
    unsafe {
        if atom_is_user_input() {
            composite.r#macro = true;
            if !atom_composite_active() {
                let lhs: [c_char; 3] = [b'@' as c_char, regname as c_char, 0];
                atom_composite_start(lhs.as_ptr(), 2);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_stuff_start(cap: *const cmdarg_T) {
    unsafe {
        if mc_replaying() || atom_composite_active() || !atom_is_user_input() {
            return;
        }
        let lhs = atom_redo_keys(atom_cmd_spec(cap));
        atom_composite_start(lhs, strlen(lhs));
        xfree(lhs.cast());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_map_start(lhs: *const c_char, len: usize, peeked: bool) {
    unsafe {
        if mc_replaying() || reg_executing != 0 || ex_normal_busy != 0 {
            return;
        }
        if (State & MODE_NORMAL) == 0 {
            atom_composite_label(lhs, len);
            return;
        }
        if atom_composite_active() {
            if get_real_state() == MODE_OP_PENDING {
                return;
            }
            typed.map_start = typed.keys.size;
            atom_composite_end();
        }
        let op = get_vim_var_str(VV_OP);
        if get_real_state() == MODE_OP_PENDING && *op != 0 {
            assert!(*lhs.add(len) == 0);
            let full = concat_str(op, lhs);
            atom_composite_start(full, strlen(full));
            xfree(full.cast());
            typed.map_start = typed.keys.size;
            return;
        }
        atom_composite_start(lhs, len);
        if peeked && !cur_frame.is_null() {
            composite.frame = (*cur_frame).id;
        }
        typed.map_start = typed.keys.size;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_visual_reset() {
    unsafe {
        vatom.state = kVatomNone;
        atoms_free(&raw mut vatom.atoms);
        vatom.origin = CmdOrigin::default();
        vatom.frame = 0;
        vatom.done = 0;
        mc_vsel_clear();
    }
}

#[inline]
unsafe fn atom_visual_pending() -> bool {
    unsafe { vatom.state != kVatomNone }
}

#[inline]
unsafe fn atom_visual_typed() -> bool {
    unsafe {
        atom_visual_pending()
            && ((vatom.state & kVatomTyped) != 0 || maptick != vatom.origin.maptick)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_visual_replayable() -> bool {
    unsafe { atom_visual_pending() && (vatom.state & kVatomVoid) == 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_visual_redoable() -> bool {
    unsafe {
        if !atom_visual_replayable() {
            return false;
        }
        for i in 0..vatom.atoms.size {
            let spec = (*vatom.atoms.items.add(i)).spec;
            if spec.cmd == b'g' as c_int && spec.cmd2 == b'v' as c_int {
                return false;
            }
        }
        true
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_visual_span() -> NvimString {
    unsafe {
        if !atom_visual_replayable() {
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        }
        let tail = atoms_concat_keys(vatom.atoms, vatom.done);
        if vatom.done == 0 {
            return tail;
        }

        let mut keys: StringBuilder = KVec::new();
        sb_concat(&mut keys, c"gv".as_ptr());
        keys.concat_len(tail.data, tail.size);
        let len = keys.size;
        keys.push(0);
        api_free_string(tail);
        NvimString {
            data: keys.items,
            size: len,
        }
    }
}

unsafe fn atom_visual_span_flush() {
    unsafe {
        if vatom.done >= vatom.atoms.size
            || !atom_visual_replayable()
            || !atom_visual_typed()
            || !mc_buf_has_cursors(curbuf)
        {
            return;
        }
        let mut edited = false;
        for i in vatom.done..vatom.atoms.size {
            assert!((*vatom.atoms.items.add(i)).r#type != kAOperator);
            edited |= (*vatom.atoms.items.add(i)).changed;
        }
        if !edited {
            return;
        }
        let span = atom_visual_span();
        vatom.done = vatom.atoms.size;
        crate::kvec::kv_push(
            &raw mut g_atoms,
            CmdAtom {
                r#type: kAVisualSpan,
                keys: span.data,
                origin: vatom.origin,
                ..CmdAtom::default()
            },
        );
    }
}

unsafe fn atom_visual_end_suffix(
    suffix: *mut c_char,
    spec: *const CmdSpec,
    redoable: bool,
) -> bool {
    unsafe {
        if mc_replaying() || atom_suppressed {
            xfree(suffix.cast());
            return false;
        }
        let origin = vatom.origin;
        let replayable = !suffix.is_null() && atom_visual_replayable();
        let v = if replayable {
            atoms_concat_keys(vatom.atoms, 0)
        } else {
            NvimString {
                data: ptr::null_mut(),
                size: 0,
            }
        };
        let prepped =
            redoable && !spec.is_null() && !suffix.is_null() && (replayable || (*spec).op != NUL);

        if prepped {
            let use_redo_keys = atom_visual_redoable();
            prep_redo_visual(
                if use_redo_keys {
                    v.data.cast_const()
                } else {
                    c"1v".as_ptr()
                },
                if use_redo_keys { v.size } else { 2 },
                CmdSpec::default(),
            );
            redo_append_str(suffix, -1);
        }

        if is_child_frame() {
            xfree(v.data.cast());
            xfree(suffix.cast());
            return prepped;
        }

        if !replayable {
            let emit = (vatom.state & kVatomVoid) != 0
                && atom_visual_typed()
                && !spec.is_null()
                && !suffix.is_null()
                && atom_is_user_cmd();
            let mut label: *mut c_char = ptr::null_mut();
            if emit {
                let collected = atoms_concat_keys(vatom.atoms, 0);
                label = xrealloc(collected.data.cast(), collected.size + strlen(suffix) + 1).cast();
                strcpy(label.add(collected.size), suffix);
            }
            xfree(suffix.cast());
            atom_visual_reset();
            if emit {
                let mut staged_atom = CmdAtom {
                    r#type: kAVisual,
                    spec: *spec,
                    keys: xstrdup(c"".as_ptr()),
                    lhs: label,
                    origin,
                    ..CmdAtom::default()
                };
                atom_stage_set(&mut staged_atom);
            }
            return prepped;
        }

        if !atom_is_user_cmd() || !atom_visual_typed() {
            xfree(v.data.cast());
            xfree(suffix.cast());
            atom_visual_reset();
            return prepped;
        }

        let keys: *mut c_char = xrealloc(v.data.cast(), v.size + strlen(suffix) + 1).cast();
        strcpy(keys.add(v.size), suffix);
        let mut atom = CmdAtom {
            r#type: kAVisual,
            spec: if !spec.is_null() {
                *spec
            } else {
                CmdSpec::default()
            },
            keys,
            origin,
            ..CmdAtom::default()
        };
        if !spec.is_null() {
            crate::kvec::kv_push(
                &raw mut vatom.atoms,
                CmdAtom {
                    r#type: kAOperator,
                    spec: *spec,
                    keys: suffix,
                    origin: (*cur_frame).origin,
                    ..CmdAtom::default()
                },
            );
        } else {
            xfree(suffix.cast());
        }
        atom.cascaded = vatom.done > 0;
        if atom.cascaded && vatom.done < vatom.atoms.size {
            let span = atom_visual_span();
            crate::kvec::kv_push(
                &raw mut g_atoms,
                CmdAtom {
                    r#type: kAVisualSpan,
                    keys: span.data,
                    origin,
                    ..CmdAtom::default()
                },
            );
        }
        atom.atoms = vatom.atoms;
        vatom.atoms = KVec::new();
        atom_visual_reset();
        atom_stage_set(&mut atom);
        prepped
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_visual_end(spec: CmdSpec, redoable: bool) -> bool {
    unsafe {
        if mc_replaying() {
            return false;
        }
        atom_visual_end_suffix(atom_redo_keys(spec), &spec, redoable)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_capture_op(oap: *mut oparg_T, cap: *mut cmdarg_T, redo_yank: bool) {
    unsafe {
        let redoable = op_redoable((*oap).op_type, redo_yank);
        let ins_op = (*oap).op_type == OP_CHANGE
            || (*oap).op_type == OP_INSERT
            || (*oap).op_type == OP_APPEND;
        let payload_motion = (*cap).cmdchar >= 0x100
            || ((*cap).cmdchar != NUL && !strchr(c"/?:!".as_ptr(), (*cap).cmdchar).is_null());
        let excmd = (*cap).cmdchar == b':' as c_int || (*cap).cmdchar == K_COMMAND;
        if !ins_op && !payload_motion {
            let prep_exempt = !redoable || (*cap).cmdchar == b'D' as c_int;
            let mut spec = CmdSpec {
                regname: (*oap).regname,
                count: c_long::from((*cap).count0),
                op: get_op_char((*oap).op_type),
                op_extra: get_extra_op_char((*oap).op_type),
                ..CmdSpec::default()
            };
            if prep_exempt && (!Visual.active || (*oap).motion_force != 0) {
                if atom_capturable(atom_buf_has_consumers(), KeyTyped) {
                    let operand = nv_is((*cap).cmdchar, NV_LANG | NV_NCH_ARG);
                    spec.motion_force = (*oap).motion_force;
                    spec.cmd = (*cap).cmdchar;
                    spec.cmd2 = if operand { NUL } else { (*cap).nchar };
                    spec.cmdarg = if operand { (*cap).nchar } else { NUL };
                    let mut op_atom = atom_from_spec(kAOperator, spec);
                    op_atom.origin = (*cur_frame).origin;
                    atom_stage_set(&mut op_atom);
                }
            } else if !Visual.active || (*oap).motion_force != 0 {
                // Prepped: atom_cmd_end() derives the atom from redobuff.
            } else if (*oap).op_type == OP_REPLACE && (*cap).nchar <= 0 {
                let suffix: [c_char; 4] = [
                    b'r' as c_char,
                    Ctrl_V as c_char,
                    if (*cap).nchar == REPLACE_CR_NCHAR {
                        CAR as c_char
                    } else {
                        NL as c_char
                    },
                    0,
                ];
                atom_visual_end_suffix(xstrdup(suffix.as_ptr()), &spec, redoable);
            } else {
                spec.cmdarg = if (*oap).op_type == OP_REPLACE {
                    (*cap).nchar
                } else {
                    NUL
                };
                if ((*oap).op_type == OP_NR_ADD || (*oap).op_type == OP_NR_SUB) && (*cap).arg != 0 {
                    spec.op_extra = spec.op;
                    spec.op = b'g' as c_int;
                }
                atom_visual_end(spec, redoable);
            }
        }

        if redoable
            && (*cap).cmdchar != b'D' as c_int
            && ((!Visual.active || (*oap).motion_force != 0) || excmd || (*cap).cmdchar == K_LUA)
        {
            prep_redo(
                true,
                false,
                CmdSpec {
                    regname: (*oap).regname,
                    count: c_long::from((*cap).count0),
                    op: get_op_char((*oap).op_type),
                    op_extra: get_extra_op_char((*oap).op_type),
                    motion_force: (*oap).motion_force,
                    cmd: (*cap).cmdchar,
                    cmd2: (*cap).nchar,
                    ..CmdSpec::default()
                },
            );
            if (*cap).cmdchar == b'/' as c_int || (*cap).cmdchar == b'?' as c_int {
                if vim_strchr(p_cpo, kCpoRedo).is_null() {
                    redo_append_lit((*cap).searchbuf, -1);
                }
                redo_append_str(c"\n".as_ptr(), 1);
            } else if excmd {
                if repeat_cmdline.is_null() {
                    redo_new(CmdSpec::default());
                } else {
                    if (*cap).cmdchar == b':' as c_int {
                        redo_append_lit(repeat_cmdline, -1);
                    } else {
                        redo_append_spec(repeat_cmdline);
                    }
                    redo_append_str(c"\n".as_ptr(), 1);
                    xfree(repeat_cmdline.cast());
                    repeat_cmdline = ptr::null_mut();
                }
            } else if (*cap).cmdchar == K_LUA {
                let mut lua_id = [0 as c_char; NUMBUFLEN];
                let n = vim_snprintf!(
                    lua_id.as_mut_ptr(),
                    lua_id.len(),
                    c"%d".as_ptr(),
                    repeat_luamap,
                );
                redo_append_str(lua_id.as_ptr(), n as isize);
                redo_append_str(c"\n".as_ptr(), 1);
            }
        } else if Visual.active && redoable && (*oap).motion_force == NUL {
            if op_self_select(cap) {
                prep_redo(
                    true,
                    false,
                    CmdSpec {
                        regname: (*oap).regname,
                        count: c_long::from((*cap).count0),
                        op: get_op_char((*oap).op_type),
                        op_extra: get_extra_op_char((*oap).op_type),
                        motion_force: (*oap).motion_force,
                        cmd: (*cap).cmdchar,
                        cmd2: (*cap).nchar,
                        ..CmdSpec::default()
                    },
                );
            } else if ins_op && !excmd && (*cap).cmdchar != K_LUA {
                let v = if atom_visual_redoable() {
                    atom_visual_span()
                } else {
                    NvimString {
                        data: ptr::null_mut(),
                        size: 0,
                    }
                };
                prep_redo_visual(
                    if !v.data.is_null() {
                        v.data.cast_const()
                    } else {
                        c"1v".as_ptr()
                    },
                    if !v.data.is_null() { v.size } else { 2 },
                    CmdSpec {
                        regname: (*oap).regname,
                        op: get_op_char((*oap).op_type),
                        op_extra: get_extra_op_char((*oap).op_type),
                        ..CmdSpec::default()
                    },
                );
                xfree(v.data.cast());
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_ins_start(
    cmd: c_int,
    count: c_long,
    vis: VisualIns,
    vblock: bool,
) -> InsSession {
    unsafe {
        let mut session = InsSession {
            typed: atom_is_user_input(),
            vis,
            origin: if vis == kVInsKeys {
                vatom.origin
            } else {
                (*cur_frame).origin
            },
            vsel: ptr::null_mut(),
        };

        if vis != kVInsNone && !mc_replaying() && !is_child_frame() {
            if vis == kVInsOther && atom_visual_replayable() && !atom_visual_redoable() {
                session.vsel = atom_visual_span().data;
                session.origin = vatom.origin;
            }
            if vis == kVInsKeys || !session.vsel.is_null() {
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
            || !session.vsel.is_null()
            || (vis == kVInsMotion && (*cur_frame).payload_start == usize::MAX);
        mc_ins_cascade_start(
            session.typed && count <= 1 && !repl && !vblock && reexec,
            session.origin,
            (*root_frame()).id,
            session.vsel,
        );

        session
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_ins_end(session: *mut InsSession, busy: bool) {
    unsafe {
        let visual = (*session).vis != kVInsNone;
        let user_input = (*session).typed || maptick != (*session).origin.maptick;

        if mc_replaying() {
            xfree((*session).vsel.cast());
            (*session).vsel = ptr::null_mut();
            return;
        }
        if mc_ins_commit() {
            if has_event(EVENT_CMDATOM) && !atom_composite_active() {
                atom_ins_push(session, false);
            }
            xfree((*session).vsel.cast());
            (*session).vsel = ptr::null_mut();
            return;
        }
        if !user_input
            || busy
            || restart_edit != 0
            || !atom_buf_has_consumers()
            || (visual && (*session).vis != kVInsKeys && (*session).vsel.is_null())
        {
            if user_input && (busy || restart_edit != 0) && atom_composite_active() {
                composite.lossy = true;
            }
            xfree((*session).vsel.cast());
            (*session).vsel = ptr::null_mut();
            return;
        }
        atom_ins_push(session, mc_buf_has_cursors(curbuf));
        xfree((*session).vsel.cast());
        (*session).vsel = ptr::null_mut();
    }
}

unsafe fn atom_ins_push(session: *const InsSession, cascade: bool) {
    unsafe {
        let mut atom = atom_from_redo(if (*session).vis != kVInsNone {
            kAVisual
        } else {
            kAInsert
        });
        if !(*session).vsel.is_null() && !atom.keys.is_null() {
            assert!(strncmp(atom.keys, c"1v".as_ptr(), 2) == 0);
            let keys = concat_str((*session).vsel, atom.keys.add(2));
            xfree(atom.keys.cast());
            atom.keys = keys;
        }
        let size = if !atom.keys.is_null() {
            strlen(atom.keys)
        } else {
            0
        };
        let replace = atom.spec.cmd == b'r' as c_int
            || (atom.spec.cmd == b'g' as c_int && atom.spec.cmd2 == b'r' as c_int);
        if size == 0 || (!replace && *atom.keys.add(size - 1) as u8 != ESC) {
            atom_free(&mut atom);
            return;
        }
        atom.text = get_last_insert_save();
        atom.origin = (*session).origin;
        atom_push_raw(cascade, &mut atom);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_cmd_start(old: *mut CmdFrame, cmdchar: c_int) {
    unsafe {
        let consumers = atom_buf_has_consumers();
        frame_id += 1;
        *old = CmdFrame {
            origin: atom_origin(),
            staged: CmdAtom::default(),
            parent: cur_frame,
            visual: Visual,
            keytyped: KeyTyped,
            keyclass: atom_key_class(cmdchar, NUL, NUL),
            ex_normal: ex_normal_busy,
            captures: atom_captures,
            global_ops,
            beeps: did_beep,
            id: frame_id,
            follow: mc_following(),
            consumers,
            reg_ts: if consumers { reg_max_ts(true) } else { 0 },
            payload_start: usize::MAX,
            payload_end: 0,
            redo_frame: 0,
            cmdline: ptr::null_mut(),
        };
        cur_frame = old;
        if (*old).parent.is_null() && !composite.open {
            composite.open = true;
            composite.origin = (*old).origin;
            composite.follow = kNone;
        }
        if (*old).parent.is_null() && !mc_replaying() && !composite.active && composite.lhs[0] != 0
        {
            if !KeyTyped && consumers {
                atom_composite_start(ptr::null(), 0);
                typed.map_start = typed.keys.size;
                composite.follow = kNone;
            } else {
                composite.lhs[0] = 0;
            }
        }
        if (*old).parent.is_null() && atom_is_user_cmd() && !atom_composite_active() {
            typed.keys.size = 0;
            typed.map_start = 0;
        }
    }
}

unsafe fn atom_capture_cmd(ca: *mut cmdarg_T, old: *mut CmdFrame) -> bool {
    unsafe {
        if mc_replaying() || atom_suppressed {
            return false;
        }

        let user = atom_is_user_cmd();
        let root = root_frame();
        let keycls = atom_key_class((*ca).cmdchar, (*ca).nchar, (*ca).extra_char);
        let opaque = (keycls & kKeyOpaque) != 0;
        let synthetic = (keycls & kKeySynthetic) != 0;
        let unchanged = curbuf == (*old).origin.buf.br_buf
            && ptr::eq(curwin.cast_const(), (*old).origin.win)
            && !atom_origin_moved((*old).origin)
            && !atom_origin_changed((*old).origin)
            && Visual.active == (*old).visual.active
            && (!Visual.active
                || (equalpos((*old).visual.start, Visual.start)
                    && (*old).visual.mode == Visual.mode));
        if opaque
            && unchanged
            && !(atom_visual_pending() && frame_id > (*old).id)
            && (*root).redo_frame != (*old).id
        {
            return false;
        }
        let ins_cascaded = user && mc_ins_cascaded((*root).id);
        let mapped = user && !(*old).keytyped && !synthetic;
        if mapped
            && global_ops == (*old).global_ops
            && ((curbuf == (*old).origin.buf.br_buf && atom_origin_changed((*old).origin))
                || ins_cascaded)
        {
            map_edit = true;
        }

        let child = is_child_frame();
        let mut vis = false;
        if Visual.active && !child {
            if !(*old).visual.active && vatom.frame <= (*old).id {
                atom_visual_reset();
                vatom.state = kVatomFed;
                vatom.origin = (*old).origin;
            }
            if atom_visual_pending()
                && user
                && !synthetic
                && ((*old).keytyped || atom_composite_active())
            {
                vatom.state = (vatom.state & kVatomVoid) | kVatomTyped;
            }
            vis = atom_visual_pending()
                && (*ca).cmdchar != b'Q' as c_int
                && (*ca).cmdchar != b'q' as c_int;
            if vis
                && (Visual.select
                    || ((keycls & (kKeyScrollMove | kKeyScrollView | kKeyMouse)) != 0
                        && !unchanged))
            {
                vatom.state |= kVatomVoid;
            }
        } else if (*old).visual.active && !child {
            let mut own = if atom_visual_pending() {
                atom_from_frame(ca, old)
            } else {
                CmdAtom::default()
            };
            if !own.keys.is_null() {
                let suffix = own.keys;
                own.keys = ptr::null_mut();
                atom_free(&mut own);
                atom_visual_end_suffix(suffix, ptr::null(), false);
            } else if user && atom_visual_replayable() && vatom.atoms.size > 0 {
                let suffix: [c_char; 2] = [ESC as c_char, 0];
                atom_visual_end_suffix(xstrdup(suffix.as_ptr()), ptr::null(), false);
            } else {
                atom_visual_end_suffix(ptr::null_mut(), ptr::null(), false);
            }
        }

        if (vis
            && vatom.frame <= (*old).id
            && atom_captures == (*old).captures
            && (*(*ca).oap).op_type == OP_NOP)
            || (!Visual.active
                && !(*old).visual.active
                && atom_capturable((*old).consumers, (*old).keytyped)
                && atom_captures == (*old).captures
                && !atom_staged()
                && (*(*ca).oap).op_type == OP_NOP
                && stuff_empty()
                && !ins_cascaded)
        {
            let collected = vatom.atoms.size;
            let special_motion = (keycls & kKeyMotion) != 0;
            let scroll_cmd = (keycls & (kKeyScrollMove | kKeyScrollView)) != 0;
            let mouse_cmd = (keycls & kKeyMouse) != 0;
            let jump_cmd = nv_is((*ca).cmdchar, NV_JUMP)
                || (keycls & kKeyJump) != 0
                || ((*ca).cmdchar == b'%' as c_int && (*ca).count0 > 0);
            let replayable = ((*ca).cmdchar > 0
                && (*ca).cmdchar < 0x100
                && (*ca).cmdchar != b'"' as c_int
                && (*ca).cmdchar != b'@' as c_int
                && !scroll_cmd)
                || special_motion;
            let changed = atom_origin_changed((*old).origin);
            let motion = (nv_is((*ca).cmdchar, NV_MOTION) || special_motion)
                && !changed
                && !finish_op
                && !jump_cmd;
            let failed = did_beep != (*old).beeps && !atom_origin_moved((*old).origin);
            let follow = (mc_following() || mapped) && motion && !failed;

            if (*root).redo_frame == (*old).id && !mouse_cmd {
                let mut atom = atom_from_redo(kAOperator);
                atom_payload_append(&mut atom, old);

                let effect = (changed
                    || reg_max_ts(true) > (*old).reg_ts
                    || (mc_following() && atom_origin_moved((*old).origin)))
                    && global_ops == (*old).global_ops;
                if !atom.keys.is_null() && *atom.keys != 0 {
                    atom.origin = (*old).origin;
                    atom_push(effect, &mut atom);
                } else {
                    atom_free(&mut atom);
                }
            } else if !(*ca).searchbuf.is_null()
                && ((*ca).cmdchar == b'/' as c_int || (*ca).cmdchar == b'?' as c_int)
                && !(vis && unchanged)
            {
                let mut atom =
                    atom_from_cmdline(kAMotion, ca, (*ca).searchbuf, (*old).visual.active);
                atom.origin = (*old).origin;
                atom_push(false, &mut atom);
            } else if (!(*root).cmdline.is_null()
                && ((*ca).cmdchar == b':' as c_int || (*ca).cmdchar == K_COMMAND))
                || (vis && (*ca).cmdchar == K_LUA)
            {
                let mut atom = atom_from_frame(ca, old);
                atom.origin = (*old).origin;
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
                atom.origin = (*old).origin;
                atom_push(follow, &mut atom);
                if jump_cmd && atom_composite_active() {
                    composite.queued = true;
                }
            } else if (scroll_cmd || mouse_cmd) && !atom_composite_active() {
                let mut spec = atom_cmd_spec(ca);
                if is_special((*ca).cmdchar) {
                    spec.count = 0;
                }
                let mut atom = atom_from_spec(if scroll_cmd { kAScroll } else { kAMouse }, spec);
                atom.origin = (*old).origin;
                atom_push(false, &mut atom);
            }
            if vis && vatom.atoms.size == collected && !unchanged {
                vatom.state |= kVatomVoid;
            }
        }

        if vis
            && (curbuf != (*old).origin.buf.br_buf
                || (atom_origin_changed((*old).origin) && vatom.frame != (*old).id))
        {
            vatom.state |= kVatomVoid;
        }
        atom_visual_span_flush();

        Visual.active && user && (*old).parent.is_null()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_cmd_end(ca: *mut cmdarg_T, old: *mut CmdFrame) {
    unsafe {
        if composite.follow == kNone
            && !mc_replaying()
            && (atom_origin_moved((*old).origin) || nv_is((*ca).cmdchar, NV_MOTION))
        {
            composite.follow = if (*old).follow { kTrue } else { kFalse };
        }
        let refresh_visual = atom_capture_cmd(ca, old);
        atom_stage_flush(old);
        if atom_composite_active()
            && (*old).payload_start != usize::MAX
            && (*old).payload_end > (*old).payload_start
        {
            composite.lossy = true;
        }

        if (*old).parent.is_null() && !mc_replaying() && typebuf_typed() != 0 && stuff_empty() {
            let map_moved = atom_composite_active() && atom_origin_moved(composite.origin);
            mc_clock_edge(map_edit, map_moved, composite.follow == kTrue);

            map_edit = false;
            if (*(*ca).oap).op_type == OP_NOP && !Visual.active && restart_edit == 0 {
                atom_composite_end();
                composite.open = false;
            }
        }
        if refresh_visual {
            mc_vsel_refresh();
        }
        if !(*old).cmdline.is_null() {
            xfree((*old).cmdline.cast());
            (*old).cmdline = ptr::null_mut();
        }
        cur_frame = (*old).parent;
    }
}
