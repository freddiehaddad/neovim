//! MessagePack-RPC session state machine (`src/mpack/rpc.c`).

use std::ffi::{c_char, c_int};
use std::mem::{offset_of, size_of, zeroed};
use std::ptr;

use crate::mpack::mpack_core::{
    MPACK_EOF, MPACK_OK, MPACK_TOKEN_ARRAY, MPACK_TOKEN_UINT, mpack_read, mpack_tokbuf_init,
    mpack_tokbuf_t, mpack_token_data_t, mpack_token_t, mpack_uint32_t, mpack_value_t, mpack_write,
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

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpack_rpc_session_t {
    pub reader: mpack_tokbuf_t,
    pub writer: mpack_tokbuf_t,
    pub receive: mpack_rpc_header_t,
    pub send: mpack_rpc_header_t,
    pub request_id: mpack_uint32_t,
    pub capacity: mpack_uint32_t,
    pub slots: [mpack_rpc_slot_s; MPACK_RPC_MAX_REQUESTS],
}

#[inline]
pub const fn mpack_rpc_session_struct_size(c: mpack_uint32_t) -> usize {
    offset_of!(mpack_rpc_session_t, slots) + (c as usize) * size_of::<mpack_rpc_slot_s>()
}

#[inline]
unsafe fn session_slots_ptr(session: *mut mpack_rpc_session_t) -> *mut mpack_rpc_slot_s {
    unsafe { (&raw mut (*session).slots).cast::<mpack_rpc_slot_s>() }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mpack_rpc_session_init(
    session: *mut mpack_rpc_session_t,
    capacity: mpack_uint32_t,
) {
    unsafe {
        (*session).capacity = if capacity != 0 {
            capacity
        } else {
            MPACK_RPC_MAX_REQUESTS as mpack_uint32_t
        };
        (*session).request_id = 0;
        mpack_tokbuf_init(&raw mut (*session).reader);
        mpack_tokbuf_init(&raw mut (*session).writer);
        mpack_rpc_reset_hdr(&raw mut (*session).receive);
        mpack_rpc_reset_hdr(&raw mut (*session).send);
        ptr::write_bytes(session_slots_ptr(session), 0, (*session).capacity as usize);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mpack_rpc_receive_tok(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mpack_rpc_request_tok(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mpack_rpc_reply_tok(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mpack_rpc_notify_tok(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mpack_rpc_receive(
    session: *mut mpack_rpc_session_t,
    buf: *mut *const c_char,
    buflen: *mut usize,
    msg: *mut mpack_rpc_message_t,
) -> c_int {
    unsafe {
        let mut status: c_int;
        loop {
            let mut tok: mpack_token_t = zeroed();
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mpack_rpc_request(
    session: *mut mpack_rpc_session_t,
    buf: *mut *mut c_char,
    buflen: *mut usize,
    data: mpack_data_t,
) -> c_int {
    unsafe {
        let mut status = MPACK_EOF;
        let mut tok: mpack_token_t = zeroed();
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mpack_rpc_reply(
    session: *mut mpack_rpc_session_t,
    buf: *mut *mut c_char,
    buflen: *mut usize,
    id: mpack_uint32_t,
) -> c_int {
    unsafe {
        let mut status = MPACK_EOF;
        let mut tok: mpack_token_t = zeroed();
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mpack_rpc_notify(
    session: *mut mpack_rpc_session_t,
    buf: *mut *mut c_char,
    buflen: *mut usize,
) -> c_int {
    unsafe {
        let mut status = MPACK_EOF;
        let mut tok: mpack_token_t = zeroed();
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mpack_rpc_session_copy(
    dst: *mut mpack_rpc_session_t,
    src: *mut mpack_rpc_session_t,
) {
    unsafe {
        let dst_capacity = (*dst).capacity;
        debug_assert!((*src).capacity <= dst_capacity);
        ptr::copy_nonoverlapping(
            src.cast::<u8>(),
            dst.cast::<u8>(),
            offset_of!(mpack_rpc_session_t, slots),
        );
        (*dst).capacity = dst_capacity;
        let dst_slots = session_slots_ptr(dst);
        let src_slots = session_slots_ptr(src);
        ptr::write_bytes(dst_slots, 0, dst_capacity as usize);
        for i in 0..((*src).capacity as usize) {
            let s = *src_slots.add(i);
            if s.used != 0 {
                mpack_rpc_put(dst, s.msg);
            }
        }
    }
}

fn mpack_rpc_request_hdr() -> mpack_rpc_header_t {
    let z = mpack_token_t {
        r#type: 0,
        length: 0,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    };
    let mut hdr = mpack_rpc_header_t {
        toks: [z; 3],
        index: 0,
    };
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
        let mut slot: *mut mpack_rpc_slot_s = ptr::null_mut();
        let cap = (*session).capacity;
        let mut hash = msg.id % cap;
        let slots = session_slots_ptr(session);

        for _ in 0..cap {
            let s = slots.add(hash as usize);
            if (*s).used == 0 || (*s).msg.id == msg.id {
                slot = s;
                break;
            }
            hash = if hash > 0 { hash - 1 } else { cap - 1 };
        }

        if slot.is_null() {
            return -1;
        }
        if (*slot).msg.id == msg.id && (*slot).used != 0 {
            return 0;
        }
        (*slot).msg = msg;
        (*slot).used = 1;
        1
    }
}

unsafe fn mpack_rpc_pop(session: *mut mpack_rpc_session_t, msg: *mut mpack_rpc_message_t) -> c_int {
    unsafe {
        let mut slot: *mut mpack_rpc_slot_s = ptr::null_mut();
        let cap = (*session).capacity;
        let mut hash = (*msg).id % cap;
        let slots = session_slots_ptr(session);

        for _ in 0..cap {
            let s = slots.add(hash as usize);
            if (*s).used != 0 && (*s).msg.id == (*msg).id {
                slot = s;
                break;
            }
            hash = if hash > 0 { hash - 1 } else { cap - 1 };
        }

        if slot.is_null() {
            return 0;
        }

        *msg = (*slot).msg;
        (*slot).used = 0;
        1
    }
}

#[inline]
unsafe fn mpack_rpc_reset_hdr(hdr: *mut mpack_rpc_header_t) {
    unsafe {
        (*hdr).index = 0;
    }
}
