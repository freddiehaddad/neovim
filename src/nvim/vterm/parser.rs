//! VTerm input byte stream parser (`src/nvim/vterm/parser.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use std::ffi::{c_char, c_int, c_long, c_void};
use std::ptr;

use crate::vterm::mouse::{CSI_LEADER_MAX, INTERMED_MAX, VTerm};

pub type VTermTerminator = c_int;

pub(crate) const VTERM_TERMINATOR_BEL: VTermTerminator = 0;
const VTERM_TERMINATOR_ST: VTermTerminator = 1;

pub(crate) const CSI_ARG_FLAG_MORE: c_long = (1u32 << 31) as c_long;
pub(crate) const CSI_ARG_MISSING: c_long = ((1u32 << 31) - 1) as c_long;

pub(crate) const NORMAL: c_int = 0;
const CSI_LEADER: c_int = 1;
const CSI_ARGS: c_int = 2;
const CSI_INTERMED: c_int = 3;
const DCS_COMMAND: c_int = 4;
const OSC_COMMAND: c_int = 5;
const OSC: c_int = 6;
const DCS_VTERM: c_int = 7;
const APC: c_int = 8;
const PM: c_int = 9;
const SOS: c_int = 10;

#[cfg(not(windows))]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermStringFragment {
    pub str_: *const c_char,
    pub packed: u32,
    pub terminator: VTermTerminator,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermStringFragment {
    pub str_: *const c_char,
    pub len_bits: usize,
    pub flags_bits: u8,
    pub terminator: VTermTerminator,
}

impl VTermStringFragment {
    #[inline]
    pub fn new(
        str_: *const c_char,
        len: usize,
        initial: bool,
        final_: bool,
        terminator: VTermTerminator,
    ) -> Self {
        #[cfg(not(windows))]
        {
            let packed = ((len as u32) & 0x3fff_ffff)
                | (u32::from(initial) << 30)
                | (u32::from(final_) << 31);
            Self {
                str_,
                packed,
                terminator,
            }
        }
        #[cfg(windows)]
        {
            let len_bits = len & 0x3fff_ffff;
            let flags_bits = u8::from(initial) | (u8::from(final_) << 1);
            Self {
                str_,
                len_bits,
                flags_bits,
                terminator,
            }
        }
    }

    #[inline]
    pub const fn len(&self) -> usize {
        #[cfg(not(windows))]
        {
            (self.packed & 0x3fff_ffff) as usize
        }
        #[cfg(windows)]
        {
            self.len_bits & 0x3fff_ffff
        }
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub const fn initial(&self) -> bool {
        #[cfg(not(windows))]
        {
            ((self.packed >> 30) & 0x01) != 0
        }
        #[cfg(windows)]
        {
            (self.flags_bits & 0x01) != 0
        }
    }

    #[inline]
    pub const fn final_(&self) -> bool {
        #[cfg(not(windows))]
        {
            ((self.packed >> 31) & 0x01) != 0
        }
        #[cfg(windows)]
        {
            ((self.flags_bits >> 1) & 0x01) != 0
        }
    }
}

#[repr(C)]
pub struct VTermParserCallbacks {
    pub text:
        Option<unsafe extern "C" fn(bytes: *const c_char, len: usize, user: *mut c_void) -> c_int>,
    pub control: Option<unsafe extern "C" fn(control: u8, user: *mut c_void) -> c_int>,
    pub escape:
        Option<unsafe extern "C" fn(bytes: *const c_char, len: usize, user: *mut c_void) -> c_int>,
    pub csi: Option<
        unsafe extern "C" fn(
            leader: *const c_char,
            args: *const c_long,
            argcount: c_int,
            intermed: *const c_char,
            command: c_char,
            user: *mut c_void,
        ) -> c_int,
    >,
    pub osc: Option<
        unsafe extern "C" fn(command: c_int, frag: VTermStringFragment, user: *mut c_void) -> c_int,
    >,
    pub dcs: Option<
        unsafe extern "C" fn(
            command: *const c_char,
            commandlen: usize,
            frag: VTermStringFragment,
            user: *mut c_void,
        ) -> c_int,
    >,
    pub apc: Option<unsafe extern "C" fn(frag: VTermStringFragment, user: *mut c_void) -> c_int>,
    pub pm: Option<unsafe extern "C" fn(frag: VTermStringFragment, user: *mut c_void) -> c_int>,
    pub sos: Option<unsafe extern "C" fn(frag: VTermStringFragment, user: *mut c_void) -> c_int>,
    pub resize: Option<unsafe extern "C" fn(rows: c_int, cols: c_int, user: *mut c_void) -> c_int>,
}

#[inline]
fn is_intermed(c: u8) -> bool {
    (0x20..=0x2f).contains(&c)
}

#[inline]
fn is_string_state(state: c_int) -> bool {
    state >= OSC_COMMAND
}

unsafe fn do_control(vt: *mut VTerm, control: u8) {
    unsafe {
        let callbacks = (*vt).parser.callbacks.cast::<VTermParserCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).control
        {
            cb(control, (*vt).parser.cbdata);
        }
    }
}

unsafe fn do_csi(vt: *mut VTerm, command: c_char) {
    unsafe {
        let callbacks = (*vt).parser.callbacks.cast::<VTermParserCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).csi
        {
            let leader = if (*vt).parser.v.csi.leaderlen != 0 {
                (*vt).parser.v.csi.leader.as_ptr()
            } else {
                ptr::null()
            };
            let intermed = if (*vt).parser.intermedlen != 0 {
                (*vt).parser.intermed.as_ptr()
            } else {
                ptr::null()
            };
            cb(
                leader,
                (*vt).parser.v.csi.args.as_ptr(),
                (*vt).parser.v.csi.argi,
                intermed,
                command,
                (*vt).parser.cbdata,
            );
        }
    }
}

unsafe fn do_escape(vt: *mut VTerm, command: c_char) {
    unsafe {
        let mut seq = [0 as c_char; INTERMED_MAX + 1];
        let mut len = (*vt).parser.intermedlen as usize;
        ptr::copy_nonoverlapping((*vt).parser.intermed.as_ptr(), seq.as_mut_ptr(), len);
        seq[len] = command;
        len += 1;
        seq[len] = 0;

        let callbacks = (*vt).parser.callbacks.cast::<VTermParserCallbacks>();
        if !callbacks.is_null()
            && let Some(cb) = (*callbacks).escape
        {
            cb(seq.as_ptr(), len, (*vt).parser.cbdata);
        }
    }
}

unsafe fn string_fragment(
    vt: *mut VTerm,
    str_: *const c_char,
    len: usize,
    final_: bool,
    terminator: VTermTerminator,
) {
    unsafe {
        let frag =
            VTermStringFragment::new(str_, len, (*vt).parser.string_initial, final_, terminator);
        let callbacks = (*vt).parser.callbacks.cast::<VTermParserCallbacks>();

        match (*vt).parser.state {
            OSC => {
                if !callbacks.is_null()
                    && let Some(cb) = (*callbacks).osc
                {
                    cb((*vt).parser.v.osc.command, frag, (*vt).parser.cbdata);
                }
            }
            DCS_VTERM => {
                if !callbacks.is_null()
                    && let Some(cb) = (*callbacks).dcs
                {
                    cb(
                        (*vt).parser.v.dcs.command.as_ptr(),
                        (*vt).parser.v.dcs.commandlen as usize,
                        frag,
                        (*vt).parser.cbdata,
                    );
                }
            }
            APC => {
                if !callbacks.is_null()
                    && let Some(cb) = (*callbacks).apc
                {
                    cb(frag, (*vt).parser.cbdata);
                }
            }
            PM => {
                if !callbacks.is_null()
                    && let Some(cb) = (*callbacks).pm
                {
                    cb(frag, (*vt).parser.cbdata);
                }
            }
            SOS => {
                if !callbacks.is_null()
                    && let Some(cb) = (*callbacks).sos
                {
                    cb(frag, (*vt).parser.cbdata);
                }
            }
            _ => return,
        }

        (*vt).parser.string_initial = false;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vterm_input_write(
    vt: *mut VTerm,
    bytes: *const c_char,
    len: usize,
) -> usize {
    unsafe {
        let mut pos: usize = 0;
        let mut string_start: *const c_char = match (*vt).parser.state {
            OSC | DCS_VTERM | APC | PM | SOS => bytes,
            _ => ptr::null(),
        };

        while pos < len {
            let mut c = *bytes.add(pos) as u8;
            let mut c1_allowed = ((*vt).mode & 0x01) == 0;

            if c == 0x00 || c == 0x7f {
                // NUL, DEL
                if is_string_state((*vt).parser.state) {
                    string_fragment(
                        vt,
                        string_start,
                        (bytes.add(pos) as usize).wrapping_sub(string_start as usize),
                        false,
                        VTERM_TERMINATOR_ST,
                    );
                    string_start = bytes.add(pos + 1);
                }
                if (*vt).parser.emit_nul {
                    do_control(vt, c);
                }
                pos += 1;
                continue;
            }
            if c == 0x18 || c == 0x1a {
                // CAN, SUB
                (*vt).parser.in_esc = 0;
                (*vt).parser.state = NORMAL;
                string_start = ptr::null();
                if (*vt).parser.emit_nul {
                    do_control(vt, c);
                }
                pos += 1;
                continue;
            } else if c == 0x1b {
                // ESC
                (*vt).parser.intermedlen = 0;
                if !is_string_state((*vt).parser.state) {
                    (*vt).parser.state = NORMAL;
                }
                (*vt).parser.in_esc = 1;
                pos += 1;
                continue;
            } else if c == 0x07 && is_string_state((*vt).parser.state) {
                // BEL, can stand for ST in OSC or DCS state
            } else if c < 0x20 {
                // other C0
                if (*vt).parser.state == SOS {
                    pos += 1;
                    continue;
                }
                if is_string_state((*vt).parser.state) {
                    string_fragment(
                        vt,
                        string_start,
                        (bytes.add(pos) as usize).wrapping_sub(string_start as usize),
                        false,
                        VTERM_TERMINATOR_ST,
                    );
                }
                do_control(vt, c);
                if is_string_state((*vt).parser.state) {
                    string_start = bytes.add(pos + 1);
                }
                pos += 1;
                continue;
            }

            let mut string_len = (bytes.add(pos) as usize).wrapping_sub(string_start as usize);

            if (*vt).parser.in_esc != 0 {
                if (*vt).parser.intermedlen == 0
                    && (0x40..0x60).contains(&c)
                    && (!is_string_state((*vt).parser.state) || c == 0x5c)
                {
                    c += 0x40;
                    c1_allowed = true;
                    string_len = string_len.saturating_sub(1);
                    (*vt).parser.in_esc = 0;
                } else {
                    string_start = ptr::null();
                    (*vt).parser.state = NORMAL;
                }
            }

            'state_machine: {
                if (*vt).parser.state == CSI_LEADER {
                    if (0x3c..=0x3f).contains(&c) {
                        if ((*vt).parser.v.csi.leaderlen as usize) < CSI_LEADER_MAX - 1 {
                            let idx = (*vt).parser.v.csi.leaderlen as usize;
                            (*vt).parser.v.csi.leader[idx] = c as c_char;
                            (*vt).parser.v.csi.leaderlen += 1;
                        }
                        break 'state_machine;
                    }

                    (*vt).parser.v.csi.leader[(*vt).parser.v.csi.leaderlen as usize] = 0;
                    (*vt).parser.v.csi.argi = 0;
                    (*vt).parser.v.csi.args[0] = CSI_ARG_MISSING;
                    (*vt).parser.state = CSI_ARGS;
                }

                if (*vt).parser.state == CSI_ARGS {
                    if c.is_ascii_digit() {
                        let argi = (*vt).parser.v.csi.argi as usize;
                        if (*vt).parser.v.csi.args[argi] == CSI_ARG_MISSING {
                            (*vt).parser.v.csi.args[argi] = 0;
                        }
                        (*vt).parser.v.csi.args[argi] *= 10;
                        (*vt).parser.v.csi.args[argi] += c_long::from(c - b'0');
                        break 'state_machine;
                    }
                    if c == b':' {
                        let argi = (*vt).parser.v.csi.argi as usize;
                        (*vt).parser.v.csi.args[argi] |= CSI_ARG_FLAG_MORE;
                        c = b';';
                    }
                    if c == b';' {
                        (*vt).parser.v.csi.argi += 1;
                        let argi = (*vt).parser.v.csi.argi as usize;
                        (*vt).parser.v.csi.args[argi] = CSI_ARG_MISSING;
                        break 'state_machine;
                    }

                    (*vt).parser.v.csi.argi += 1;
                    (*vt).parser.intermedlen = 0;
                    (*vt).parser.state = CSI_INTERMED;
                }

                if (*vt).parser.state == CSI_INTERMED {
                    if is_intermed(c) {
                        if ((*vt).parser.intermedlen as usize) < INTERMED_MAX - 1 {
                            let idx = (*vt).parser.intermedlen as usize;
                            (*vt).parser.intermed[idx] = c as c_char;
                            (*vt).parser.intermedlen += 1;
                        }
                        break 'state_machine;
                    } else if c == 0x1b {
                        // ESC in CSI cancels
                    } else if (0x40..=0x7e).contains(&c) {
                        (*vt).parser.intermed[(*vt).parser.intermedlen as usize] = 0;
                        do_csi(vt, c as c_char);
                    }

                    (*vt).parser.state = NORMAL;
                    string_start = ptr::null();
                    break 'state_machine;
                }

                if (*vt).parser.state == OSC_COMMAND {
                    if c.is_ascii_digit() {
                        if (*vt).parser.v.osc.command == -1 {
                            (*vt).parser.v.osc.command = 0;
                        } else {
                            (*vt).parser.v.osc.command *= 10;
                        }
                        (*vt).parser.v.osc.command += c_int::from(c - b'0');
                        break 'state_machine;
                    }
                    if c == b';' {
                        (*vt).parser.state = OSC;
                        string_start = bytes.add(pos + 1);
                        break 'state_machine;
                    }

                    string_start = bytes.add(pos);
                    string_len = 0;
                    (*vt).parser.state = OSC;
                    // Fall through to string_state
                }

                if (*vt).parser.state == DCS_COMMAND {
                    if ((*vt).parser.v.dcs.commandlen as usize) < CSI_LEADER_MAX {
                        let idx = (*vt).parser.v.dcs.commandlen as usize;
                        (*vt).parser.v.dcs.command[idx] = c as c_char;
                        (*vt).parser.v.dcs.commandlen += 1;
                    }

                    if (0x40..=0x7e).contains(&c) {
                        string_start = bytes.add(pos + 1);
                        (*vt).parser.state = DCS_VTERM;
                    }
                    break 'state_machine;
                }

                match (*vt).parser.state {
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
                            (*vt).parser.state = NORMAL;
                            string_start = ptr::null();
                        }
                    }
                    NORMAL => {
                        if (*vt).parser.in_esc != 0 {
                            if is_intermed(c) {
                                if ((*vt).parser.intermedlen as usize) < INTERMED_MAX - 1 {
                                    let idx = (*vt).parser.intermedlen as usize;
                                    (*vt).parser.intermed[idx] = c as c_char;
                                    (*vt).parser.intermedlen += 1;
                                }
                            } else if (0x30..0x7f).contains(&c) {
                                do_escape(vt, c as c_char);
                                (*vt).parser.in_esc = 0;
                                (*vt).parser.state = NORMAL;
                                string_start = ptr::null();
                            }
                            break 'state_machine;
                        }
                        if c1_allowed && (0x80..0xa0).contains(&c) {
                            match c {
                                0x90 => {
                                    // DCS
                                    (*vt).parser.string_initial = true;
                                    (*vt).parser.v.dcs.commandlen = 0;
                                    (*vt).parser.state = DCS_COMMAND;
                                    string_start = ptr::null();
                                }
                                0x98 => {
                                    // SOS
                                    (*vt).parser.string_initial = true;
                                    (*vt).parser.state = SOS;
                                    string_start = bytes.add(pos + 1);
                                }
                                0x9b => {
                                    // CSI
                                    (*vt).parser.v.csi.leaderlen = 0;
                                    (*vt).parser.state = CSI_LEADER;
                                    string_start = ptr::null();
                                }
                                0x9d => {
                                    // OSC
                                    (*vt).parser.v.osc.command = -1;
                                    (*vt).parser.string_initial = true;
                                    (*vt).parser.state = OSC_COMMAND;
                                    string_start = ptr::null();
                                }
                                0x9e => {
                                    // PM
                                    (*vt).parser.string_initial = true;
                                    (*vt).parser.state = PM;
                                    string_start = bytes.add(pos + 1);
                                }
                                0x9f => {
                                    // APC
                                    (*vt).parser.string_initial = true;
                                    (*vt).parser.state = APC;
                                    string_start = bytes.add(pos + 1);
                                }
                                _ => {
                                    do_control(vt, c);
                                }
                            }
                        } else {
                            let mut eaten: usize = 0;
                            let callbacks = (*vt).parser.callbacks.cast::<VTermParserCallbacks>();
                            if !callbacks.is_null()
                                && let Some(cb) = (*callbacks).text
                            {
                                eaten = cb(bytes.add(pos), len - pos, (*vt).parser.cbdata) as usize;
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
            let mut string_len = (bytes.add(pos) as usize).wrapping_sub(string_start as usize);
            if string_len > 0 {
                if (*vt).parser.in_esc != 0 {
                    string_len -= 1;
                }
                string_fragment(vt, string_start, string_len, false, VTERM_TERMINATOR_ST);
            }
        }

        len
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vterm_parser_set_callbacks(
    vt: *mut VTerm,
    callbacks: *const VTermParserCallbacks,
    user: *mut c_void,
) {
    unsafe {
        (*vt).parser.callbacks = callbacks.cast();
        (*vt).parser.cbdata = user;
    }
}
