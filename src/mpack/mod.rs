//! MessagePack core, object parser, RPC session, and Lua bindings (`src/mpack/`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

pub mod conv;
pub mod lmpack;
pub mod mpack_core;
pub mod object;
pub mod rpc;

#[cfg(test)]
mod tests {
    use std::ffi::c_char;

    use crate::mpack::conv::{
        mpack_pack_boolean, mpack_pack_number, mpack_pack_sint, mpack_pack_uint,
        mpack_unpack_boolean, mpack_unpack_number, mpack_unpack_sint, mpack_unpack_uint,
    };
    use crate::mpack::mpack_core::{
        MPACK_OK, mpack_read, mpack_tokbuf_init, mpack_tokbuf_t, mpack_token_t, mpack_write,
    };

    #[test]
    fn test_mpack_conv_and_core_roundtrip() {
        let tb = mpack_pack_boolean(1);
        assert!(mpack_unpack_boolean(tb));

        let tu = mpack_pack_uint(0x1234_5678_9abc_def0);
        assert_eq!(mpack_unpack_uint(tu), 0x1234_5678_9abc_def0);

        let ts = mpack_pack_sint(-42);
        let mut ts_with_len = ts;
        ts_with_len.length = 8;
        assert_eq!(mpack_unpack_sint(ts_with_len), -42);

        let tn = mpack_pack_number(-1234.5);
        assert!((mpack_unpack_number(tn) - (-1234.5)).abs() < f64::EPSILON);

        unsafe {
            let mut wbuf = [0 as c_char; 32];
            let mut wptr = wbuf.as_mut_ptr();
            let mut wlen = wbuf.len();
            let mut wtb: mpack_tokbuf_t = std::mem::zeroed();
            mpack_tokbuf_init(&mut wtb);
            assert_eq!(mpack_write(&mut wtb, &mut wptr, &mut wlen, &tu), MPACK_OK);

            let written = wbuf.len() - wlen;
            let mut rptr = wbuf.as_ptr();
            let mut rlen = written;
            let mut rtb: mpack_tokbuf_t = std::mem::zeroed();
            mpack_tokbuf_init(&mut rtb);
            let mut out_tok: mpack_token_t = std::mem::zeroed();
            assert_eq!(
                mpack_read(&mut rtb, &mut rptr, &mut rlen, &mut out_tok),
                MPACK_OK
            );
            assert_eq!(mpack_unpack_uint(out_tok), 0x1234_5678_9abc_def0);
        }
    }
}
