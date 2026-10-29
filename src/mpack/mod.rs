//! MessagePack core, object parser, RPC session, and Lua bindings (`src/mpack/`).

pub mod conv;
pub mod lmpack;
pub mod mpack_core;
pub mod object;
pub mod rpc;

#[cfg(test)]
mod tests {
    use crate::mpack::conv::{
        mpack_pack_boolean, mpack_pack_number, mpack_unpack_boolean, mpack_unpack_number,
        mpack_unpack_sint, mpack_unpack_uint,
    };
    use crate::mpack::mpack_core::{
        MPACK_OK, MpackTokbuf, MpackToken, mpack_read, mpack_tokbuf_init, mpack_write,
    };

    #[test]
    fn test_mpack_conv_and_core_roundtrip() {
        let tb = mpack_pack_boolean(1);
        assert!(mpack_unpack_boolean(tb));
        assert!(!mpack_unpack_boolean(mpack_pack_boolean(0)));

        let tu = mpack_pack_number(0x1_0000_0005u64 as f64);
        assert_eq!(mpack_unpack_uint(tu), 0x1_0000_0005);

        let ts = mpack_pack_number(-123456.0);
        let mut ts_with_len = ts;
        ts_with_len.length = 8;
        assert_eq!(mpack_unpack_sint(ts_with_len), -123456);

        let tn = mpack_pack_number(-1234.5);
        assert!((mpack_unpack_number(tn) - (-1234.5)).abs() < f64::EPSILON);

        let mut wbuf = [0u8; 32];
        let mut wptr = wbuf.as_mut_ptr();
        let mut wlen = wbuf.len();
        let mut wtb = MpackTokbuf::new();
        mpack_tokbuf_init(&mut wtb);
        assert_eq!(mpack_write(&mut wtb, &mut wptr, &mut wlen, &tu), MPACK_OK);

        let written = wbuf.len() - wlen;
        let mut rptr = wbuf.as_ptr();
        let mut rlen = written;
        let mut rtb = MpackTokbuf::new();
        mpack_tokbuf_init(&mut rtb);
        let mut out_tok = MpackToken::new();
        assert_eq!(
            mpack_read(&mut rtb, &mut rptr, &mut rlen, &mut out_tok),
            MPACK_OK
        );
        assert_eq!(mpack_unpack_uint(out_tok), 0x1_0000_0005);

        for (val, expected_len) in [(-128.0, 1), (-129.0, 2), (-32768.0, 2), (-32769.0, 4)] {
            let tok = mpack_pack_number(val);
            assert_eq!(
                tok.r#type,
                crate::mpack::mpack_core::MpackTokenType::MpackTokenSint
            );
            assert_eq!(tok.length, expected_len);
        }

        let mut p = crate::mpack::object::MpackParser::new(2);
        crate::mpack::object::mpack_parser_init(&mut p, 2);
        assert_eq!(p.capacity, 2);
        crate::mpack::object::mpack_parser_grow(&mut p);
        assert_eq!(p.capacity, 4);

        let mut s = crate::mpack::rpc::MpackRpcSession::new(2);
        assert_eq!(s.slots.len(), 2);
        crate::mpack::rpc::mpack_rpc_session_grow(&mut s);
        assert_eq!(s.slots.len(), 4);
    }
}
