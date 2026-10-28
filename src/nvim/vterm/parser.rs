//! VTerm input byte stream parser (`src/nvim/vterm/parser.c`).

use std::ffi::c_char;
use std::ptr;

use crate::vterm::keyboard::VTermStateFull;
use crate::vterm::mouse::{CSI_LEADER_MAX, INTERMED_MAX, VTerm};

pub type VTermTerminator = i32;

pub(crate) const VTERM_TERMINATOR_BEL: VTermTerminator = 0;
const VTERM_TERMINATOR_ST: VTermTerminator = 1;

pub(crate) const CSI_ARG_FLAG_MORE: i64 = (1u32 << 31) as i64;
pub(crate) const CSI_ARG_MISSING: i64 = ((1u32 << 31) - 1) as i64;

pub(crate) const NORMAL: i32 = 0;
const CSI_LEADER: i32 = 1;
const CSI_ARGS: i32 = 2;
const CSI_INTERMED: i32 = 3;
const DCS_COMMAND: i32 = 4;
const OSC_COMMAND: i32 = 5;
const OSC: i32 = 6;
const DCS_VTERM: i32 = 7;
const APC: i32 = 8;
const PM: i32 = 9;
const SOS: i32 = 10;

#[derive(Clone, Copy)]
pub struct VTermStringFragment {
    pub str_: *const c_char,
    len: usize,
    initial: bool,
    final_: bool,
    pub terminator: VTermTerminator,
}

impl VTermStringFragment {
    #[inline]
    pub const fn new(
        str_: *const c_char,
        len: usize,
        initial: bool,
        final_: bool,
        terminator: VTermTerminator,
    ) -> Self {
        Self {
            str_,
            len,
            initial,
            final_,
            terminator,
        }
    }

    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub const fn initial(&self) -> bool {
        self.initial
    }

    #[inline]
    pub const fn final_(&self) -> bool {
        self.final_
    }
}

pub type VTermCsiFn<T> = fn(
    leader: *const c_char,
    args: *const i64,
    argcount: i32,
    intermed: *const c_char,
    command: c_char,
    user: *mut T,
) -> i32;

pub type VTermDcsFn<T> =
    fn(command: *const c_char, commandlen: usize, frag: VTermStringFragment, user: *mut T) -> i32;

#[repr(C)]
pub struct VTermParserCallbacks {
    pub text: Option<fn(bytes: *const c_char, len: usize, user: *mut VTermStateFull) -> i32>,
    pub control: Option<fn(control: u8, user: *mut VTermStateFull) -> i32>,
    pub escape: Option<fn(bytes: *const c_char, len: usize, user: *mut VTermStateFull) -> i32>,
    pub csi: Option<VTermCsiFn<VTermStateFull>>,
    pub osc: Option<fn(command: i32, frag: VTermStringFragment, user: *mut VTermStateFull) -> i32>,
    pub dcs: Option<VTermDcsFn<VTermStateFull>>,
    pub apc: Option<fn(frag: VTermStringFragment, user: *mut VTermStateFull) -> i32>,
    pub pm: Option<fn(frag: VTermStringFragment, user: *mut VTermStateFull) -> i32>,
    pub sos: Option<fn(frag: VTermStringFragment, user: *mut VTermStateFull) -> i32>,
    pub resize: Option<fn(rows: i32, cols: i32, user: *mut VTermStateFull) -> i32>,
}

#[inline]
fn is_intermed(c: u8) -> bool {
    (0x20..=0x2f).contains(&c)
}

#[inline]
fn is_string_state(state: i32) -> bool {
    state >= OSC_COMMAND
}

fn do_control(vt: &mut VTerm, control: u8) {
    if let Some(callbacks) = vt.parser.callbacks
        && let Some(cb) = callbacks.control
    {
        cb(control, vt.state_mut_ptr());
    }
}

fn do_csi(vt: &mut VTerm, command: c_char) {
    if let Some(callbacks) = vt.parser.callbacks
        && let Some(cb) = callbacks.csi
    {
        let leader = if vt.parser.csi.leaderlen != 0 {
            vt.parser.csi.leader.as_ptr()
        } else {
            ptr::null()
        };
        let intermed = if vt.parser.intermedlen != 0 {
            vt.parser.intermed.as_ptr()
        } else {
            ptr::null()
        };
        cb(
            leader,
            vt.parser.csi.args.as_ptr(),
            vt.parser.csi.argi,
            intermed,
            command,
            vt.state_mut_ptr(),
        );
    }
}

fn do_escape(vt: &mut VTerm, command: c_char) {
    let mut seq = [0 as c_char; INTERMED_MAX + 1];
    let mut len = vt.parser.intermedlen as usize;
    unsafe { ptr::copy_nonoverlapping(vt.parser.intermed.as_ptr(), seq.as_mut_ptr(), len) };
    seq[len] = command;
    len += 1;
    seq[len] = 0;

    if let Some(callbacks) = vt.parser.callbacks
        && let Some(cb) = callbacks.escape
    {
        cb(seq.as_ptr(), len, vt.state_mut_ptr());
    }
}

fn string_fragment(
    vt: &mut VTerm,
    str_: impl crate::strings::IntoCCharPtr,
    len: usize,
    final_: bool,
    terminator: VTermTerminator,
) {
    let str_ = str_.into_c_char_ptr();
    let frag = VTermStringFragment::new(str_, len, vt.parser.string_initial, final_, terminator);
    let callbacks = vt.parser.callbacks;
    let state = vt.state_mut_ptr();

    match vt.parser.state {
        OSC => {
            if let Some(cbs) = callbacks
                && let Some(cb) = cbs.osc
            {
                cb(vt.parser.osc.command, frag, state);
            }
        }
        DCS_VTERM => {
            if let Some(cbs) = callbacks
                && let Some(cb) = cbs.dcs
            {
                cb(
                    vt.parser.dcs.command.as_ptr(),
                    vt.parser.dcs.commandlen as usize,
                    frag,
                    state,
                );
            }
        }
        APC => {
            if let Some(cbs) = callbacks
                && let Some(cb) = cbs.apc
            {
                cb(frag, state);
            }
        }
        PM => {
            if let Some(cbs) = callbacks
                && let Some(cb) = cbs.pm
            {
                cb(frag, state);
            }
        }
        SOS => {
            if let Some(cbs) = callbacks
                && let Some(cb) = cbs.sos
            {
                cb(frag, state);
            }
        }
        _ => return,
    }

    vt.parser.string_initial = false;
}

pub fn vterm_input_write(
    vt: &mut VTerm,
    bytes: impl crate::strings::IntoCCharPtr,
    len: usize,
) -> usize {
    let bytes = bytes.into_c_char_ptr();
    {
        let mut pos: usize = 0;
        let mut string_start: *const c_char = match vt.parser.state {
            OSC | DCS_VTERM | APC | PM | SOS => bytes,
            _ => ptr::null(),
        };

        while pos < len {
            let mut c = crate::memory::cstr_byte_at(bytes, pos);
            let mut c1_allowed = (vt.mode & 0x01) == 0;

            if c == 0x00 || c == 0x7f {
                // NUL, DEL
                if is_string_state(vt.parser.state) {
                    string_fragment(
                        vt,
                        string_start,
                        (bytes.wrapping_add(pos) as usize).wrapping_sub(string_start as usize),
                        false,
                        VTERM_TERMINATOR_ST,
                    );
                    string_start = bytes.wrapping_add(pos + 1);
                }
                if vt.parser.emit_nul {
                    do_control(vt, c);
                }
                pos += 1;
                continue;
            }
            if c == 0x18 || c == 0x1a {
                // CAN, SUB
                vt.parser.in_esc = 0;
                vt.parser.state = NORMAL;
                string_start = ptr::null();
                if vt.parser.emit_nul {
                    do_control(vt, c);
                }
                pos += 1;
                continue;
            } else if c == 0x1b {
                // ESC
                vt.parser.intermedlen = 0;
                if !is_string_state(vt.parser.state) {
                    vt.parser.state = NORMAL;
                }
                vt.parser.in_esc = 1;
                pos += 1;
                continue;
            } else if c == 0x07 && is_string_state(vt.parser.state) {
                // BEL, can stand for ST in OSC or DCS state
            } else if c < 0x20 {
                // other C0
                if vt.parser.state == SOS {
                    pos += 1;
                    continue;
                }
                if is_string_state(vt.parser.state) {
                    string_fragment(
                        vt,
                        string_start,
                        (bytes.wrapping_add(pos) as usize).wrapping_sub(string_start as usize),
                        false,
                        VTERM_TERMINATOR_ST,
                    );
                }
                do_control(vt, c);
                if is_string_state(vt.parser.state) {
                    string_start = bytes.wrapping_add(pos + 1);
                }
                pos += 1;
                continue;
            }

            let mut string_len =
                (bytes.wrapping_add(pos) as usize).wrapping_sub(string_start as usize);

            if vt.parser.in_esc != 0 {
                if vt.parser.intermedlen == 0
                    && (0x40..0x60).contains(&c)
                    && (!is_string_state(vt.parser.state) || c == 0x5c)
                {
                    c += 0x40;
                    c1_allowed = true;
                    string_len = string_len.saturating_sub(1);
                    vt.parser.in_esc = 0;
                } else {
                    string_start = ptr::null();
                    vt.parser.state = NORMAL;
                }
            }

            'state_machine: {
                if vt.parser.state == CSI_LEADER {
                    if (0x3c..=0x3f).contains(&c) {
                        if (vt.parser.csi.leaderlen as usize) < CSI_LEADER_MAX - 1 {
                            let idx = vt.parser.csi.leaderlen as usize;
                            vt.parser.csi.leader[idx] = c as c_char;
                            vt.parser.csi.leaderlen += 1;
                        }
                        break 'state_machine;
                    }

                    vt.parser.csi.leader[vt.parser.csi.leaderlen as usize] = 0;
                    vt.parser.csi.argi = 0;
                    vt.parser.csi.args[0] = CSI_ARG_MISSING;
                    vt.parser.state = CSI_ARGS;
                }

                if vt.parser.state == CSI_ARGS {
                    if c.is_ascii_digit() {
                        let argi = vt.parser.csi.argi as usize;
                        if vt.parser.csi.args[argi] == CSI_ARG_MISSING {
                            vt.parser.csi.args[argi] = 0;
                        }
                        vt.parser.csi.args[argi] *= 10;
                        vt.parser.csi.args[argi] += i64::from(c - b'0');
                        break 'state_machine;
                    }
                    if c == b':' {
                        let argi = vt.parser.csi.argi as usize;
                        vt.parser.csi.args[argi] |= CSI_ARG_FLAG_MORE;
                        c = b';';
                    }
                    if c == b';' {
                        vt.parser.csi.argi += 1;
                        let argi = vt.parser.csi.argi as usize;
                        vt.parser.csi.args[argi] = CSI_ARG_MISSING;
                        break 'state_machine;
                    }

                    vt.parser.csi.argi += 1;
                    vt.parser.intermedlen = 0;
                    vt.parser.state = CSI_INTERMED;
                }

                if vt.parser.state == CSI_INTERMED {
                    if is_intermed(c) {
                        if (vt.parser.intermedlen as usize) < INTERMED_MAX - 1 {
                            let idx = vt.parser.intermedlen as usize;
                            vt.parser.intermed[idx] = c as c_char;
                            vt.parser.intermedlen += 1;
                        }
                        break 'state_machine;
                    } else if c == 0x1b {
                        // ESC in CSI cancels
                    } else if (0x40..=0x7e).contains(&c) {
                        vt.parser.intermed[vt.parser.intermedlen as usize] = 0;
                        do_csi(vt, c as c_char);
                    }

                    vt.parser.state = NORMAL;
                    string_start = ptr::null();
                    break 'state_machine;
                }

                if vt.parser.state == OSC_COMMAND {
                    if c.is_ascii_digit() {
                        if vt.parser.osc.command == -1 {
                            vt.parser.osc.command = 0;
                        } else {
                            vt.parser.osc.command *= 10;
                        }
                        vt.parser.osc.command += i32::from(c - b'0');
                        break 'state_machine;
                    }
                    if c == b';' {
                        vt.parser.state = OSC;
                        string_start = bytes.wrapping_add(pos + 1);
                        break 'state_machine;
                    }

                    string_start = bytes.wrapping_add(pos);
                    string_len = 0;
                    vt.parser.state = OSC;
                    // Fall through to string_state
                }

                if vt.parser.state == DCS_COMMAND {
                    if (vt.parser.dcs.commandlen as usize) < CSI_LEADER_MAX {
                        let idx = vt.parser.dcs.commandlen as usize;
                        vt.parser.dcs.command[idx] = c as c_char;
                        vt.parser.dcs.commandlen += 1;
                    }

                    if (0x40..=0x7e).contains(&c) {
                        string_start = bytes.wrapping_add(pos + 1);
                        vt.parser.state = DCS_VTERM;
                    }
                    break 'state_machine;
                }

                match vt.parser.state {
                    OSC | DCS_VTERM | APC | PM | SOS => {
                        if c == 0x07 || (c1_allowed && c == 0x9c) {
                            string_fragment(
                                vt,
                                string_start,
                                string_len,
                                true,
                                if c == 0x07 {
                                    VTERM_TERMINATOR_BEL
                                } else {
                                    VTERM_TERMINATOR_ST
                                },
                            );
                            vt.parser.state = NORMAL;
                            string_start = ptr::null();
                        }
                    }
                    NORMAL => {
                        if vt.parser.in_esc != 0 {
                            if is_intermed(c) {
                                if (vt.parser.intermedlen as usize) < INTERMED_MAX - 1 {
                                    let idx = vt.parser.intermedlen as usize;
                                    vt.parser.intermed[idx] = c as c_char;
                                    vt.parser.intermedlen += 1;
                                }
                            } else if (0x30..0x7f).contains(&c) {
                                do_escape(vt, c as c_char);
                                vt.parser.in_esc = 0;
                                vt.parser.state = NORMAL;
                                string_start = ptr::null();
                            }
                            break 'state_machine;
                        }
                        if c1_allowed && (0x80..0xa0).contains(&c) {
                            match c {
                                0x90 => {
                                    // DCS
                                    vt.parser.string_initial = true;
                                    vt.parser.dcs.commandlen = 0;
                                    vt.parser.state = DCS_COMMAND;
                                    string_start = ptr::null();
                                }
                                0x98 => {
                                    // SOS
                                    vt.parser.string_initial = true;
                                    vt.parser.state = SOS;
                                    string_start = bytes.wrapping_add(pos + 1);
                                }
                                0x9b => {
                                    // CSI
                                    vt.parser.csi.leaderlen = 0;
                                    vt.parser.state = CSI_LEADER;
                                    string_start = ptr::null();
                                }
                                0x9d => {
                                    // OSC
                                    vt.parser.osc.command = -1;
                                    vt.parser.string_initial = true;
                                    vt.parser.state = OSC_COMMAND;
                                    string_start = ptr::null();
                                }
                                0x9e => {
                                    // PM
                                    vt.parser.string_initial = true;
                                    vt.parser.state = PM;
                                    string_start = bytes.wrapping_add(pos + 1);
                                }
                                0x9f => {
                                    // APC
                                    vt.parser.string_initial = true;
                                    vt.parser.state = APC;
                                    string_start = bytes.wrapping_add(pos + 1);
                                }
                                _ => {
                                    do_control(vt, c);
                                }
                            }
                        } else {
                            let mut eaten: usize = 0;
                            if let Some(callbacks) = vt.parser.callbacks
                                && let Some(cb) = callbacks.text
                            {
                                eaten = cb(bytes.wrapping_add(pos), len - pos, vt.state_mut_ptr())
                                    as usize;
                            }

                            if eaten == 0 {
                                eaten = 1;
                            }

                            pos += eaten - 1;
                        }
                    }
                    _ => {}
                }
            }

            pos += 1;
        }

        if !string_start.is_null() {
            let mut string_len =
                (bytes.wrapping_add(pos) as usize).wrapping_sub(string_start as usize);
            if string_len > 0 {
                if vt.parser.in_esc != 0 {
                    string_len -= 1;
                }
                string_fragment(vt, string_start, string_len, false, VTERM_TERMINATOR_ST);
            }
        }

        len
    }
}

pub fn vterm_parser_set_callbacks(
    vt: &mut VTerm,
    callbacks: Option<&'static VTermParserCallbacks>,
) {
    vt.parser.callbacks = callbacks;
}
