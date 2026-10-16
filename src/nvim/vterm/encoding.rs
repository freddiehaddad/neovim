//! VTerm character encodings (`src/nvim/vterm/encoding.c`).

use std::ffi::{c_char, c_int, c_void};

const UNICODE_INVALID: u32 = 0xFFFD;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum VTermEncodingType {
    ENC_UTF8 = 0,
    ENC_SINGLE_94 = 1,
}

#[repr(C)]
pub struct VTermEncoding {
    pub init: Option<unsafe extern "C" fn(enc: *mut VTermEncoding, data: *mut c_void)>,
    pub decode: Option<
        unsafe extern "C" fn(
            enc: *mut VTermEncoding,
            data: *mut c_void,
            cp: *mut u32,
            cpi: *mut c_int,
            cplen: c_int,
            bytes: *const c_char,
            pos: *mut usize,
            bytelen: usize,
        ),
    >,
}

// `VTermEncoding` contains only function pointers and static tables.
unsafe impl Sync for VTermEncoding {}

#[repr(C)]
struct UTF8DecoderData {
    bytes_remaining: c_int,
    bytes_total: c_int,
    this_cp: c_int,
}

unsafe extern "C" fn init_utf8(_enc: *mut VTermEncoding, data_: *mut c_void) {
    let data = unsafe { &mut *data_.cast::<UTF8DecoderData>() };
    data.bytes_remaining = 0;
    data.bytes_total = 0;
}

unsafe extern "C" fn decode_utf8(
    _enc: *mut VTermEncoding,
    data_: *mut c_void,
    cp: *mut u32,
    cpi: *mut c_int,
    cplen: c_int,
    bytes: *const c_char,
    pos: *mut usize,
    bytelen: usize,
) {
    let data = unsafe { &mut *data_.cast::<UTF8DecoderData>() };

    unsafe {
        while *pos < bytelen && *cpi < cplen {
            let c = *bytes.add(*pos) as u8;

            if c < 0x20 || c == 0x7f {
                return;
            } else if (0x20..0x7f).contains(&c) {
                if data.bytes_remaining != 0 {
                    *cp.add(*cpi as usize) = UNICODE_INVALID;
                    *cpi += 1;
                }
                *cp.add(*cpi as usize) = u32::from(c);
                *cpi += 1;
                data.bytes_remaining = 0;
            } else if (0x80..0xc0).contains(&c) {
                if data.bytes_remaining == 0 {
                    *cp.add(*cpi as usize) = UNICODE_INVALID;
                    *cpi += 1;
                    *pos += 1;
                    continue;
                }

                data.this_cp = (data.this_cp << 6) | c_int::from(c & 0x3f);
                data.bytes_remaining -= 1;

                if data.bytes_remaining == 0 {
                    match data.bytes_total {
                        2 if data.this_cp < 0x0080 => data.this_cp = UNICODE_INVALID as c_int,
                        3 if data.this_cp < 0x0800 => data.this_cp = UNICODE_INVALID as c_int,
                        4 if data.this_cp < 0x10000 => data.this_cp = UNICODE_INVALID as c_int,
                        5 if data.this_cp < 0x200000 => data.this_cp = UNICODE_INVALID as c_int,
                        6 if data.this_cp < 0x4000000 => data.this_cp = UNICODE_INVALID as c_int,
                        _ => {}
                    }
                    if (0xD800..=0xDFFF).contains(&data.this_cp)
                        || data.this_cp == 0xFFFE
                        || data.this_cp == 0xFFFF
                    {
                        data.this_cp = UNICODE_INVALID as c_int;
                    }
                    *cp.add(*cpi as usize) = data.this_cp as u32;
                    *cpi += 1;
                }
            } else if (0xc0..0xe0).contains(&c) {
                if data.bytes_remaining != 0 {
                    *cp.add(*cpi as usize) = UNICODE_INVALID;
                    *cpi += 1;
                }
                data.this_cp = c_int::from(c & 0x1f);
                data.bytes_total = 2;
                data.bytes_remaining = 1;
            } else if (0xe0..0xf0).contains(&c) {
                if data.bytes_remaining != 0 {
                    *cp.add(*cpi as usize) = UNICODE_INVALID;
                    *cpi += 1;
                }
                data.this_cp = c_int::from(c & 0x0f);
                data.bytes_total = 3;
                data.bytes_remaining = 2;
            } else if (0xf0..0xf8).contains(&c) {
                if data.bytes_remaining != 0 {
                    *cp.add(*cpi as usize) = UNICODE_INVALID;
                    *cpi += 1;
                }
                data.this_cp = c_int::from(c & 0x07);
                data.bytes_total = 4;
                data.bytes_remaining = 3;
            } else if (0xf8..0xfc).contains(&c) {
                if data.bytes_remaining != 0 {
                    *cp.add(*cpi as usize) = UNICODE_INVALID;
                    *cpi += 1;
                }
                data.this_cp = c_int::from(c & 0x03);
                data.bytes_total = 5;
                data.bytes_remaining = 4;
            } else if (0xfc..0xfe).contains(&c) {
                if data.bytes_remaining != 0 {
                    *cp.add(*cpi as usize) = UNICODE_INVALID;
                    *cpi += 1;
                }
                data.this_cp = c_int::from(c & 0x01);
                data.bytes_total = 6;
                data.bytes_remaining = 5;
            } else {
                *cp.add(*cpi as usize) = UNICODE_INVALID;
                *cpi += 1;
            }

            *pos += 1;
        }
    }
}

static ENCODING_UTF8: VTermEncoding = VTermEncoding {
    init: Some(init_utf8),
    decode: Some(decode_utf8),
};

unsafe extern "C" fn decode_usascii(
    _enc: *mut VTermEncoding,
    _data: *mut c_void,
    cp: *mut u32,
    cpi: *mut c_int,
    cplen: c_int,
    bytes: *const c_char,
    pos: *mut usize,
    bytelen: usize,
) {
    unsafe {
        let is_gr = (*bytes.add(*pos) as u8) & 0x80;

        while *pos < bytelen && *cpi < cplen {
            let c = (*bytes.add(*pos) as u8) ^ is_gr;
            if c < 0x20 || c == 0x7f || c >= 0x80 {
                return;
            }
            *cp.add(*cpi as usize) = u32::from(c);
            *cpi += 1;
            *pos += 1;
        }
    }
}

static ENCODING_USASCII: VTermEncoding = VTermEncoding {
    init: None,
    decode: Some(decode_usascii),
};

#[repr(C)]
struct StaticTableEncoding {
    enc: VTermEncoding,
    chars: [u32; 128],
}

unsafe impl Sync for StaticTableEncoding {}

unsafe extern "C" fn decode_table(
    enc: *mut VTermEncoding,
    _data: *mut c_void,
    cp: *mut u32,
    cpi: *mut c_int,
    cplen: c_int,
    bytes: *const c_char,
    pos: *mut usize,
    bytelen: usize,
) {
    unsafe {
        let table = &*enc.cast::<StaticTableEncoding>();
        let is_gr = (*bytes.add(*pos) as u8) & 0x80;

        while *pos < bytelen && *cpi < cplen {
            let c = (*bytes.add(*pos) as u8) ^ is_gr;
            if c < 0x20 || c == 0x7f || c >= 0x80 {
                return;
            }
            let mapped = table.chars[c as usize];
            *cp.add(*cpi as usize) = if mapped != 0 { mapped } else { u32::from(c) };
            *cpi += 1;
            *pos += 1;
        }
    }
}

static ENCODING_DEC_DRAWING: StaticTableEncoding = {
    let mut chars = [0u32; 128];
    chars[0x60] = 0x25C6;
    chars[0x61] = 0x2592;
    chars[0x62] = 0x2409;
    chars[0x63] = 0x240C;
    chars[0x64] = 0x240D;
    chars[0x65] = 0x240A;
    chars[0x66] = 0x00B0;
    chars[0x67] = 0x00B1;
    chars[0x68] = 0x2424;
    chars[0x69] = 0x240B;
    chars[0x6a] = 0x2518;
    chars[0x6b] = 0x2510;
    chars[0x6c] = 0x250C;
    chars[0x6d] = 0x2514;
    chars[0x6e] = 0x253C;
    chars[0x6f] = 0x23BA;
    chars[0x70] = 0x23BB;
    chars[0x71] = 0x2500;
    chars[0x72] = 0x23BC;
    chars[0x73] = 0x23BD;
    chars[0x74] = 0x251C;
    chars[0x75] = 0x2524;
    chars[0x76] = 0x2534;
    chars[0x77] = 0x252C;
    chars[0x78] = 0x2502;
    chars[0x79] = 0x2A7D;
    chars[0x7a] = 0x2A7E;
    chars[0x7b] = 0x03C0;
    chars[0x7c] = 0x2260;
    chars[0x7d] = 0x00A3;
    chars[0x7e] = 0x00B7;

    StaticTableEncoding {
        enc: VTermEncoding {
            init: None,
            decode: Some(decode_table),
        },
        chars,
    }
};

/// Look up a `VTermEncoding` by `(type, designation)` (`vterm_lookup_encoding`).
#[unsafe(no_mangle)]
pub extern "C" fn vterm_lookup_encoding(
    enc_type: VTermEncodingType,
    designation: c_char,
) -> *mut VTermEncoding {
    match (enc_type, designation as u8) {
        (VTermEncodingType::ENC_UTF8, b'u') => (&raw const ENCODING_UTF8).cast_mut(),
        (VTermEncodingType::ENC_SINGLE_94, b'0') => (&raw const ENCODING_DEC_DRAWING)
            .cast::<VTermEncoding>()
            .cast_mut(),
        (VTermEncodingType::ENC_SINGLE_94, b'B') => (&raw const ENCODING_USASCII).cast_mut(),
        _ => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_and_decode_utf8_and_dec_drawing() {
        let utf8 = vterm_lookup_encoding(VTermEncodingType::ENC_UTF8, b'u' as c_char);
        assert!(!utf8.is_null());
        let dec = vterm_lookup_encoding(VTermEncodingType::ENC_SINGLE_94, b'0' as c_char);
        assert!(!dec.is_null());
        let ascii = vterm_lookup_encoding(VTermEncodingType::ENC_SINGLE_94, b'B' as c_char);
        assert!(!ascii.is_null());
        assert!(vterm_lookup_encoding(VTermEncodingType::ENC_UTF8, b'X' as c_char).is_null());

        unsafe {
            let mut state = [0u32; 4];
            if let Some(init) = (*utf8).init {
                init(utf8, state.as_mut_ptr().cast::<c_void>());
            }
            let decode = (*utf8).decode.unwrap();
            let input = "Aé€𐍈";
            let mut cp = [0u32; 8];
            let mut cpi: c_int = 0;
            let mut pos: usize = 0;
            decode(
                utf8,
                state.as_mut_ptr().cast::<c_void>(),
                cp.as_mut_ptr(),
                &mut cpi,
                8,
                input.as_ptr().cast::<c_char>(),
                &mut pos,
                input.len(),
            );
            assert_eq!(pos, input.len());
            assert_eq!(cpi, 4);
            assert_eq!(&cp[..4], &[0x41, 0x00E9, 0x20AC, 0x10348]);

            // DEC Special Graphics 'q' (0x71) -> 0x2500 BOX DRAWINGS LIGHT HORIZONTAL
            let dec_decode = (*dec).decode.unwrap();
            let dec_in = b"Aq";
            let mut dec_cp = [0u32; 4];
            let mut dec_cpi: c_int = 0;
            let mut dec_pos: usize = 0;
            dec_decode(
                dec,
                std::ptr::null_mut(),
                dec_cp.as_mut_ptr(),
                &mut dec_cpi,
                4,
                dec_in.as_ptr().cast::<c_char>(),
                &mut dec_pos,
                dec_in.len(),
            );
            assert_eq!(dec_cpi, 2);
            assert_eq!(&dec_cp[..2], &[0x41, 0x2500]);
        }
    }
}
