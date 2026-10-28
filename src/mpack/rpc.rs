//! MessagePack-RPC session state machine (`src/mpack/rpc.c`).

use std::ffi::{c_char, c_int};

use crate::mpack::mpack_core::{
    MPACK_EOF, MPACK_OK, MpackTokenArray, MpackTokenUint, mpack_read, mpack_tokbuf_init,
    MpackTokbuf, MpackToken, MpackUint32, mpack_write,
};
use crate::mpack::object::{MPACK_NOMEM, MpackData};

pub const MPACK_RPC_MAX_REQUESTS: usize = 32;

pub const MPACK_RPC_REQUEST: c_int = MPACK_NOMEM + 1;
pub const MPACK_RPC_RESPONSE: c_int = MPACK_RPC_REQUEST + 1;
pub const MPACK_RPC_NOTIFICATION: c_int = MPACK_RPC_REQUEST + 2;
pub const MPACK_RPC_ERROR: c_int = MPACK_RPC_REQUEST + 3;

pub const MPACK_RPC_EARRAY: c_int = MPACK_RPC_ERROR;
pub const MPACK_RPC_EARRAYL: c_int = MPACK_RPC_ERROR + 1;
pub const MPACK_RPC_ETYPE: c_int = MPACK_RPC_ERROR + 2;
pub const MPACK_RPC_EMSGID: c_int = MPACK_RPC_ERROR + 3;
pub const MPACK_RPC_ERESPID: c_int = MPACK_RPC_ERROR + 4;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct MpackRpcHeader {
    pub toks: [MpackToken; 3],
    pub index: c_int,
}

impl MpackRpcHeader {
    pub const fn new() -> Self {
        Self {
            toks: [MpackToken::new(); 3],
            index: 0,
        }
    }
}

impl Default for MpackRpcHeader {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct MpackRpcMessage {
    pub id: MpackUint32,
    pub data: MpackData,
}

#[repr(C)]
#[derive(Clone)]
pub struct MpackRpcSlot {
    pub used: c_int,
    pub msg: MpackRpcMessage,
}

pub struct MpackRpcSession {
    pub reader: MpackTokbuf,
    pub writer: MpackTokbuf,
    pub receive: MpackRpcHeader,
    pub send: MpackRpcHeader,
    pub request_id: MpackUint32,
    pub slots: Vec<MpackRpcSlot>,
}

impl MpackRpcSession {
    pub fn new(capacity: MpackUint32) -> Self {
        let cap = if capacity != 0 {
            capacity as usize
        } else {
            MPACK_RPC_MAX_REQUESTS
        };
        let empty_slot = MpackRpcSlot {
            used: 0,
            msg: MpackRpcMessage {
                id: 0,
                data: MpackData::new(),
            },
        };
        let mut session = MpackRpcSession {
            reader: MpackTokbuf::new(),
            writer: MpackTokbuf::new(),
            receive: MpackRpcHeader::new(),
            send: MpackRpcHeader::new(),
            request_id: 0,
            slots: vec![empty_slot; cap],
        };
        mpack_tokbuf_init(&mut session.reader);
        mpack_tokbuf_init(&mut session.writer);
        mpack_rpc_reset_hdr(&mut session.receive);
        mpack_rpc_reset_hdr(&mut session.send);
        session
    }
}

pub fn mpack_rpc_session_grow(session: &mut MpackRpcSession) {
    let new_cap = session.slots.len() * 2;
    let empty_slot = MpackRpcSlot {
        used: 0,
        msg: MpackRpcMessage {
            id: 0,
            data: MpackData::new(),
        },
    };
    let old_slots = std::mem::replace(&mut session.slots, vec![empty_slot; new_cap]);
    for s in old_slots {
        if s.used != 0 {
            mpack_rpc_put(session, s.msg);
        }
    }
}

pub fn mpack_rpc_receive_tok(
    session: &mut MpackRpcSession,
    tok: MpackToken,
    msg: &mut MpackRpcMessage,
) -> c_int {
    if session.receive.index == 0 {
        if tok.r#type != MpackTokenArray {
            return MPACK_RPC_EARRAY;
        }
        if tok.length < 3 || tok.length > 4 {
            return MPACK_RPC_EARRAYL;
        }
        session.receive.toks[0] = tok;
        session.receive.index += 1;
        return MPACK_EOF;
    }

    if session.receive.index == 1 {
        let v_lo = tok.data.value().lo;
        if tok.r#type != MpackTokenUint || tok.length > 1 || v_lo > 2 {
            return MPACK_RPC_ETYPE;
        }
        if v_lo < 2 && session.receive.toks[0].length != 4 {
            return MPACK_RPC_EARRAYL;
        }
        if v_lo == 2 && session.receive.toks[0].length != 3 {
            return MPACK_RPC_EARRAYL;
        }

        session.receive.toks[1] = tok;
        session.receive.index += 1;

        if v_lo < 2 {
            return MPACK_EOF;
        }

        mpack_rpc_reset_hdr(&mut session.receive);
        return MPACK_RPC_NOTIFICATION;
    }

    debug_assert_eq!(session.receive.index, 2);

    if tok.r#type != MpackTokenUint || tok.length > 4 {
        return MPACK_RPC_EMSGID;
    }

    msg.id = tok.data.value().lo;
    msg.data = MpackData::Null;
    let r#type = (session.receive.toks[1].data.value().lo as c_int) + MPACK_RPC_REQUEST;

    if r#type == MPACK_RPC_RESPONSE && mpack_rpc_pop(session, msg) == 0 {
        return MPACK_RPC_ERESPID;
    }

    mpack_rpc_reset_hdr(&mut session.receive);
    r#type
}

pub fn mpack_rpc_request_tok(
    session: &mut MpackRpcSession,
    tok: &mut MpackToken,
    data: MpackData,
) -> c_int {
    if session.send.index == 0 {
        loop {
            let msg = MpackRpcMessage {
                id: session.request_id,
                data: data.clone(),
            };
            session.send = mpack_rpc_request_hdr();
            session.send.toks[2].r#type = MpackTokenUint;
            session.send.toks[2].data.value_mut().lo = msg.id;
            session.send.toks[2].data.value_mut().hi = 0;
            *tok = session.send.toks[0];
            let status = mpack_rpc_put(session, msg);
            if status == -1 {
                return MPACK_NOMEM;
            }
            session.request_id = (session.request_id + 1) % 0xffffffff;
            if status != 0 {
                break;
            }
        }
        session.send.index += 1;
        return MPACK_EOF;
    }

    if session.send.index == 1 {
        *tok = session.send.toks[1];
        session.send.index += 1;
        return MPACK_EOF;
    }

    debug_assert_eq!(session.send.index, 2);
    *tok = session.send.toks[2];
    mpack_rpc_reset_hdr(&mut session.send);
    MPACK_OK
}

pub fn mpack_rpc_reply_tok(
    session: &mut MpackRpcSession,
    tok: &mut MpackToken,
    id: MpackUint32,
) -> c_int {
    if session.send.index == 0 {
        session.send = mpack_rpc_reply_hdr();
        session.send.toks[2].r#type = MpackTokenUint;
        session.send.toks[2].data.value_mut().lo = id;
        session.send.toks[2].data.value_mut().hi = 0;
        *tok = session.send.toks[0];
        session.send.index += 1;
        return MPACK_EOF;
    }

    if session.send.index == 1 {
        *tok = session.send.toks[1];
        session.send.index += 1;
        return MPACK_EOF;
    }

    debug_assert_eq!(session.send.index, 2);
    *tok = session.send.toks[2];
    mpack_rpc_reset_hdr(&mut session.send);
    MPACK_OK
}

pub fn mpack_rpc_notify_tok(
    session: &mut MpackRpcSession,
    tok: &mut MpackToken,
) -> c_int {
    if session.send.index == 0 {
        session.send = mpack_rpc_notify_hdr();
        *tok = session.send.toks[0];
        session.send.index += 1;
        return MPACK_EOF;
    }

    debug_assert_eq!(session.send.index, 1);
    *tok = session.send.toks[1];
    mpack_rpc_reset_hdr(&mut session.send);
    MPACK_OK
}

pub fn mpack_rpc_receive(
    session: &mut MpackRpcSession,
    buf: &mut *const c_char,
    buflen: &mut usize,
    msg: &mut MpackRpcMessage,
) -> c_int {
    let mut status: c_int;
    loop {
        let mut tok = MpackToken::new();
        status = mpack_read(&mut session.reader, buf, buflen, &mut tok);
        if status != 0 {
            break;
        }
        status = mpack_rpc_receive_tok(session, tok, msg);
        if status >= MPACK_RPC_REQUEST || *buflen == 0 {
            break;
        }
    }
    status
}

pub fn mpack_rpc_request(
    session: &mut MpackRpcSession,
    buf: &mut *mut c_char,
    buflen: &mut usize,
    data: MpackData,
) -> c_int {
    let mut status = MPACK_EOF;
    let mut tok = MpackToken::new();
    while status != 0 && *buflen != 0 {
        if session.writer.plen == 0 {
            status = mpack_rpc_request_tok(session, &mut tok, data.clone());
        }
        if status == MPACK_NOMEM {
            break;
        }
        let write_status = mpack_write(&mut session.writer, buf, buflen, &tok);
        if write_status != 0 {
            status = write_status;
        }
    }
    status
}

pub fn mpack_rpc_reply(
    session: &mut MpackRpcSession,
    buf: &mut *mut c_char,
    buflen: &mut usize,
    id: MpackUint32,
) -> c_int {
    let mut status = MPACK_EOF;
    let mut tok = MpackToken::new();
    while status != 0 && *buflen != 0 {
        if session.writer.plen == 0 {
            status = mpack_rpc_reply_tok(session, &mut tok, id);
        }
        let write_status = mpack_write(&mut session.writer, buf, buflen, &tok);
        if write_status != 0 {
            status = write_status;
        }
    }
    status
}

pub fn mpack_rpc_notify(
    session: &mut MpackRpcSession,
    buf: &mut *mut c_char,
    buflen: &mut usize,
) -> c_int {
    let mut status = MPACK_EOF;
    let mut tok = MpackToken::new();
    while status != 0 && *buflen != 0 {
        if session.writer.plen == 0 {
            status = mpack_rpc_notify_tok(session, &mut tok);
        }
        let write_status = mpack_write(&mut session.writer, buf, buflen, &tok);
        if write_status != 0 {
            status = write_status;
        }
    }
    status
}

fn mpack_rpc_request_hdr() -> MpackRpcHeader {
    let mut hdr = MpackRpcHeader::new();
    hdr.toks[0].r#type = MpackTokenArray;
    hdr.toks[0].length = 4;
    hdr.toks[1].r#type = MpackTokenUint;
    hdr.toks[1].data.value_mut().lo = 0;
    hdr.toks[1].data.value_mut().hi = 0;
    hdr
}

fn mpack_rpc_reply_hdr() -> MpackRpcHeader {
    let mut hdr = mpack_rpc_request_hdr();
    hdr.toks[1].data.value_mut().lo = 1;
    hdr.toks[1].data.value_mut().hi = 0;
    hdr
}

fn mpack_rpc_notify_hdr() -> MpackRpcHeader {
    let mut hdr = mpack_rpc_request_hdr();
    hdr.toks[0].length = 3;
    hdr.toks[1].data.value_mut().lo = 2;
    hdr.toks[1].data.value_mut().hi = 0;
    hdr
}

fn mpack_rpc_put(session: &mut MpackRpcSession, msg: MpackRpcMessage) -> c_int {
    let slots = &mut session.slots;
    let cap = slots.len() as MpackUint32;
    let mut hash = msg.id % cap;
    let mut found: Option<usize> = None;

    for _ in 0..cap {
        let idx = hash as usize;
        if slots[idx].used == 0 || slots[idx].msg.id == msg.id {
            found = Some(idx);
            break;
        }
        hash = if hash > 0 { hash - 1 } else { cap - 1 };
    }

    let Some(idx) = found else {
        return -1;
    };
    if slots[idx].msg.id == msg.id && slots[idx].used != 0 {
        return 0;
    }
    slots[idx].msg = msg;
    slots[idx].used = 1;
    1
}

fn mpack_rpc_pop(session: &mut MpackRpcSession, msg: &mut MpackRpcMessage) -> c_int {
    let slots = &mut session.slots;
    let cap = slots.len() as MpackUint32;
    let mut hash = msg.id % cap;
    let mut found: Option<usize> = None;

    for _ in 0..cap {
        let idx = hash as usize;
        if slots[idx].used != 0 && slots[idx].msg.id == msg.id {
            found = Some(idx);
            break;
        }
        hash = if hash > 0 { hash - 1 } else { cap - 1 };
    }

    let Some(idx) = found else {
        return 0;
    };

    *msg = slots[idx].msg.clone();
    slots[idx].used = 0;
    1
}

#[inline]
fn mpack_rpc_reset_hdr(hdr: &mut MpackRpcHeader) {
    hdr.index = 0;
}
