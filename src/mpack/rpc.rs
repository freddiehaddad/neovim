//! MessagePack-RPC session state machine (`src/mpack/rpc.c`).

use std::ffi::{c_char, c_int};
use std::ptr;

use crate::mpack::mpack_core::{
    MPACK_EOF, MPACK_OK, MPACK_TOKEN_ARRAY, MPACK_TOKEN_UINT, mpack_read, mpack_tokbuf_init,
    mpack_tokbuf_t, mpack_token_t, mpack_uint32_t, mpack_write,
};
use crate::mpack::object::{MPACK_NOMEM, mpack_data_t};

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
pub struct mpack_rpc_header_t {
    pub toks: [mpack_token_t; 3],
    pub index: c_int,
}

impl mpack_rpc_header_t {
    pub const fn new() -> Self {
        Self {
            toks: [mpack_token_t::new(); 3],
            index: 0,
        }
    }
}

impl Default for mpack_rpc_header_t {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpack_rpc_message_t {
    pub id: mpack_uint32_t,
    pub data: mpack_data_t,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpack_rpc_slot_s {
    pub used: c_int,
    pub msg: mpack_rpc_message_t,
}

pub struct mpack_rpc_session_t {
    pub reader: mpack_tokbuf_t,
    pub writer: mpack_tokbuf_t,
    pub receive: mpack_rpc_header_t,
    pub send: mpack_rpc_header_t,
    pub request_id: mpack_uint32_t,
    pub slots: Vec<mpack_rpc_slot_s>,
}

pub unsafe fn mpack_rpc_session_new(capacity: mpack_uint32_t) -> *mut mpack_rpc_session_t {
    unsafe {
        let cap = if capacity != 0 {
            capacity as usize
        } else {
            MPACK_RPC_MAX_REQUESTS
        };
        let empty_slot = mpack_rpc_slot_s {
            used: 0,
            msg: mpack_rpc_message_t {
                id: 0,
                data: mpack_data_t::new(),
            },
        };
        let mut session = Box::new(mpack_rpc_session_t {
            reader: mpack_tokbuf_t::new(),
            writer: mpack_tokbuf_t::new(),
            receive: mpack_rpc_header_t::new(),
            send: mpack_rpc_header_t::new(),
            request_id: 0,
            slots: vec![empty_slot; cap],
        });
        mpack_tokbuf_init(&raw mut session.reader);
        mpack_tokbuf_init(&raw mut session.writer);
        mpack_rpc_reset_hdr(&raw mut session.receive);
        mpack_rpc_reset_hdr(&raw mut session.send);
        Box::into_raw(session)
    }
}

pub unsafe fn mpack_rpc_session_free(session: *mut mpack_rpc_session_t) {
    if !session.is_null() {
        unsafe {
            drop(Box::from_raw(session));
        }
    }
}

pub unsafe fn mpack_rpc_session_grow(session: *mut mpack_rpc_session_t) {
    unsafe {
        let new_cap = (*session).slots.len() * 2;
        let empty_slot = mpack_rpc_slot_s {
            used: 0,
            msg: mpack_rpc_message_t {
                id: 0,
                data: mpack_data_t::new(),
            },
        };
        let old_slots = std::mem::replace(&mut (*session).slots, vec![empty_slot; new_cap]);
        for s in old_slots {
            if s.used != 0 {
                mpack_rpc_put(session, s.msg);
            }
        }
    }
}

pub unsafe fn mpack_rpc_receive_tok(
    session: *mut mpack_rpc_session_t,
    tok: mpack_token_t,
    msg: *mut mpack_rpc_message_t,
) -> c_int {
    unsafe {
        if (*session).receive.index == 0 {
            if tok.r#type != MPACK_TOKEN_ARRAY {
                return MPACK_RPC_EARRAY;
            }
            if tok.length < 3 || tok.length > 4 {
                return MPACK_RPC_EARRAYL;
            }
            (*session).receive.toks[0] = tok;
            (*session).receive.index += 1;
            return MPACK_EOF;
        }

        if (*session).receive.index == 1 {
            if tok.r#type != MPACK_TOKEN_UINT || tok.length > 1 || tok.data.value.lo > 2 {
                return MPACK_RPC_ETYPE;
            }
            if tok.data.value.lo < 2 && (*session).receive.toks[0].length != 4 {
                return MPACK_RPC_EARRAYL;
            }
            if tok.data.value.lo == 2 && (*session).receive.toks[0].length != 3 {
                return MPACK_RPC_EARRAYL;
            }

            (*session).receive.toks[1] = tok;
            (*session).receive.index += 1;

            if tok.data.value.lo < 2 {
                return MPACK_EOF;
            }

            mpack_rpc_reset_hdr(&raw mut (*session).receive);
            return MPACK_RPC_NOTIFICATION;
        }

        debug_assert_eq!((*session).receive.index, 2);

        if tok.r#type != MPACK_TOKEN_UINT || tok.length > 4 {
            return MPACK_RPC_EMSGID;
        }

        (*msg).id = tok.data.value.lo;
        (*msg).data.p = ptr::null_mut();
        let r#type = ((*session).receive.toks[1].data.value.lo as c_int) + MPACK_RPC_REQUEST;

        if r#type == MPACK_RPC_RESPONSE && mpack_rpc_pop(session, msg) == 0 {
            return MPACK_RPC_ERESPID;
        }

        mpack_rpc_reset_hdr(&raw mut (*session).receive);
        r#type
    }
}

pub unsafe fn mpack_rpc_request_tok(
    session: *mut mpack_rpc_session_t,
    tok: *mut mpack_token_t,
    data: mpack_data_t,
) -> c_int {
    unsafe {
        if (*session).send.index == 0 {
            loop {
                let msg = mpack_rpc_message_t {
                    id: (*session).request_id,
                    data,
                };
                (*session).send = mpack_rpc_request_hdr();
                (*session).send.toks[2].r#type = MPACK_TOKEN_UINT;
                (*session).send.toks[2].data.value.lo = msg.id;
                (*session).send.toks[2].data.value.hi = 0;
                *tok = (*session).send.toks[0];
                let status = mpack_rpc_put(session, msg);
                if status == -1 {
                    return MPACK_NOMEM;
                }
                (*session).request_id = ((*session).request_id + 1) % 0xffffffff;
                if status != 0 {
                    break;
                }
            }
            (*session).send.index += 1;
            return MPACK_EOF;
        }

        if (*session).send.index == 1 {
            *tok = (*session).send.toks[1];
            (*session).send.index += 1;
            return MPACK_EOF;
        }

        debug_assert_eq!((*session).send.index, 2);
        *tok = (*session).send.toks[2];
        mpack_rpc_reset_hdr(&raw mut (*session).send);
        MPACK_OK
    }
}

pub unsafe fn mpack_rpc_reply_tok(
    session: *mut mpack_rpc_session_t,
    tok: *mut mpack_token_t,
    id: mpack_uint32_t,
) -> c_int {
    unsafe {
        if (*session).send.index == 0 {
            (*session).send = mpack_rpc_reply_hdr();
            (*session).send.toks[2].r#type = MPACK_TOKEN_UINT;
            (*session).send.toks[2].data.value.lo = id;
            (*session).send.toks[2].data.value.hi = 0;
            *tok = (*session).send.toks[0];
            (*session).send.index += 1;
            return MPACK_EOF;
        }

        if (*session).send.index == 1 {
            *tok = (*session).send.toks[1];
            (*session).send.index += 1;
            return MPACK_EOF;
        }

        debug_assert_eq!((*session).send.index, 2);
        *tok = (*session).send.toks[2];
        mpack_rpc_reset_hdr(&raw mut (*session).send);
        MPACK_OK
    }
}

pub unsafe fn mpack_rpc_notify_tok(
    session: *mut mpack_rpc_session_t,
    tok: *mut mpack_token_t,
) -> c_int {
    unsafe {
        if (*session).send.index == 0 {
            (*session).send = mpack_rpc_notify_hdr();
            *tok = (*session).send.toks[0];
            (*session).send.index += 1;
            return MPACK_EOF;
        }

        debug_assert_eq!((*session).send.index, 1);
        *tok = (*session).send.toks[1];
        mpack_rpc_reset_hdr(&raw mut (*session).send);
        MPACK_OK
    }
}

pub unsafe fn mpack_rpc_receive(
    session: *mut mpack_rpc_session_t,
    buf: *mut *const c_char,
    buflen: *mut usize,
    msg: *mut mpack_rpc_message_t,
) -> c_int {
    unsafe {
        let mut status: c_int;
        loop {
            let mut tok = mpack_token_t::new();
            status = mpack_read(&raw mut (*session).reader, buf, buflen, &mut tok);
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
}

pub unsafe fn mpack_rpc_request(
    session: *mut mpack_rpc_session_t,
    buf: *mut *mut c_char,
    buflen: *mut usize,
    data: mpack_data_t,
) -> c_int {
    unsafe {
        let mut status = MPACK_EOF;
        let mut tok = mpack_token_t::new();
        while status != 0 && *buflen != 0 {
            if (*session).writer.plen == 0 {
                status = mpack_rpc_request_tok(session, &mut tok, data);
            }
            if status == MPACK_NOMEM {
                break;
            }
            let write_status = mpack_write(&raw mut (*session).writer, buf, buflen, &tok);
            if write_status != 0 {
                status = write_status;
            }
        }
        status
    }
}

pub unsafe fn mpack_rpc_reply(
    session: *mut mpack_rpc_session_t,
    buf: *mut *mut c_char,
    buflen: *mut usize,
    id: mpack_uint32_t,
) -> c_int {
    unsafe {
        let mut status = MPACK_EOF;
        let mut tok = mpack_token_t::new();
        while status != 0 && *buflen != 0 {
            if (*session).writer.plen == 0 {
                status = mpack_rpc_reply_tok(session, &mut tok, id);
            }
            let write_status = mpack_write(&raw mut (*session).writer, buf, buflen, &tok);
            if write_status != 0 {
                status = write_status;
            }
        }
        status
    }
}

pub unsafe fn mpack_rpc_notify(
    session: *mut mpack_rpc_session_t,
    buf: *mut *mut c_char,
    buflen: *mut usize,
) -> c_int {
    unsafe {
        let mut status = MPACK_EOF;
        let mut tok = mpack_token_t::new();
        while status != 0 && *buflen != 0 {
            if (*session).writer.plen == 0 {
                status = mpack_rpc_notify_tok(session, &mut tok);
            }
            let write_status = mpack_write(&raw mut (*session).writer, buf, buflen, &tok);
            if write_status != 0 {
                status = write_status;
            }
        }
        status
    }
}

fn mpack_rpc_request_hdr() -> mpack_rpc_header_t {
    let mut hdr = mpack_rpc_header_t::new();
    hdr.toks[0].r#type = MPACK_TOKEN_ARRAY;
    hdr.toks[0].length = 4;
    hdr.toks[1].r#type = MPACK_TOKEN_UINT;
    hdr.toks[1].data.value.lo = 0;
    hdr.toks[1].data.value.hi = 0;
    hdr
}

fn mpack_rpc_reply_hdr() -> mpack_rpc_header_t {
    let mut hdr = mpack_rpc_request_hdr();
    hdr.toks[1].data.value.lo = 1;
    hdr.toks[1].data.value.hi = 0;
    hdr
}

fn mpack_rpc_notify_hdr() -> mpack_rpc_header_t {
    let mut hdr = mpack_rpc_request_hdr();
    hdr.toks[0].length = 3;
    hdr.toks[1].data.value.lo = 2;
    hdr.toks[1].data.value.hi = 0;
    hdr
}

unsafe fn mpack_rpc_put(session: *mut mpack_rpc_session_t, msg: mpack_rpc_message_t) -> c_int {
    unsafe {
        let slots = &mut (*session).slots;
        let cap = slots.len() as mpack_uint32_t;
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
}

unsafe fn mpack_rpc_pop(session: *mut mpack_rpc_session_t, msg: *mut mpack_rpc_message_t) -> c_int {
    unsafe {
        let slots = &mut (*session).slots;
        let cap = slots.len() as mpack_uint32_t;
        let mut hash = (*msg).id % cap;
        let mut found: Option<usize> = None;

        for _ in 0..cap {
            let idx = hash as usize;
            if slots[idx].used != 0 && slots[idx].msg.id == (*msg).id {
                found = Some(idx);
                break;
            }
            hash = if hash > 0 { hash - 1 } else { cap - 1 };
        }

        let Some(idx) = found else {
            return 0;
        };

        *msg = slots[idx].msg;
        slots[idx].used = 0;
        1
    }
}

#[inline]
unsafe fn mpack_rpc_reset_hdr(hdr: *mut mpack_rpc_header_t) {
    unsafe {
        (*hdr).index = 0;
    }
}
