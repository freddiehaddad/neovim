//! VTerm character encodings (`src/nvim/vterm/encoding.c`).

use std::ffi::{c_char, c_int};

const UNICODE_INVALID: u32 = 0xFFFD;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VTermEncodingType {
    ENC_UTF8 = 0,
    ENC_SINGLE_94 = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VTermEncoding {
    Utf8,
    UsAscii,
    DecDrawing,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct VTermEncodingInstance {
    pub enc: Option<VTermEncoding>,
    bytes_remaining: c_int,
    bytes_total: c_int,
    this_cp: c_int,
}

const DEC_DRAWING_CHARS: [u32; 128] = {
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
    chars
};

impl VTermEncodingInstance {
    #[inline]
    pub fn init(&mut self, enc: VTermEncoding) {
        self.enc = Some(enc);
        self.bytes_remaining = 0;
        self.bytes_total = 0;
        self.this_cp = 0;
    }

    pub fn decode(&mut self, cp: &mut [u32], cpi: &mut c_int, bytes: &[u8], pos: &mut usize) {
        let Some(enc) = self.enc else {
            return;
        };
        let cplen = cp.len() as c_int;
        let bytelen = bytes.len();

        match enc {
            VTermEncoding::Utf8 => {
                while *pos < bytelen && *cpi < cplen {
                    let c = bytes[*pos];

                    if c < 0x20 || c == 0x7f {
                        return;
                    } else if (0x20..0x7f).contains(&c) {
                        if self.bytes_remaining != 0 {
                            cp[*cpi as usize] = UNICODE_INVALID;
                            *cpi += 1;
                        }
                        if *cpi < cplen {
                            cp[*cpi as usize] = u32::from(c);
                            *cpi += 1;
                        }
                        self.bytes_remaining = 0;
                    } else if (0x80..0xc0).contains(&c) {
                        if self.bytes_remaining == 0 {
                            cp[*cpi as usize] = UNICODE_INVALID;
                            *cpi += 1;
                            *pos += 1;
                            continue;
                        }

                        self.this_cp = (self.this_cp << 6) | c_int::from(c & 0x3f);
                        self.bytes_remaining -= 1;

                        if self.bytes_remaining == 0 {
                            match self.bytes_total {
                                2 if self.this_cp < 0x0080 => {
                                    self.this_cp = UNICODE_INVALID as c_int
                                }
                                3 if self.this_cp < 0x0800 => {
                                    self.this_cp = UNICODE_INVALID as c_int
                                }
                                4 if self.this_cp < 0x10000 => {
                                    self.this_cp = UNICODE_INVALID as c_int
                                }
                                5 if self.this_cp < 0x200000 => {
                                    self.this_cp = UNICODE_INVALID as c_int
                                }
                                6 if self.this_cp < 0x4000000 => {
                                    self.this_cp = UNICODE_INVALID as c_int
                                }
                                _ => {}
                            }
                            if (0xD800..=0xDFFF).contains(&self.this_cp)
                                || self.this_cp == 0xFFFE
                                || self.this_cp == 0xFFFF
                            {
                                self.this_cp = UNICODE_INVALID as c_int;
                            }
                            cp[*cpi as usize] = self.this_cp as u32;
                            *cpi += 1;
                        }
                    } else if (0xc0..0xe0).contains(&c) {
                        if self.bytes_remaining != 0 {
                            cp[*cpi as usize] = UNICODE_INVALID;
                            *cpi += 1;
                        }
                        self.this_cp = c_int::from(c & 0x1f);
                        self.bytes_total = 2;
                        self.bytes_remaining = 1;
                    } else if (0xe0..0xf0).contains(&c) {
                        if self.bytes_remaining != 0 {
                            cp[*cpi as usize] = UNICODE_INVALID;
                            *cpi += 1;
                        }
                        self.this_cp = c_int::from(c & 0x0f);
                        self.bytes_total = 3;
                        self.bytes_remaining = 2;
                    } else if (0xf0..0xf8).contains(&c) {
                        if self.bytes_remaining != 0 {
                            cp[*cpi as usize] = UNICODE_INVALID;
                            *cpi += 1;
                        }
                        self.this_cp = c_int::from(c & 0x07);
                        self.bytes_total = 4;
                        self.bytes_remaining = 3;
                    } else if (0xf8..0xfc).contains(&c) {
                        if self.bytes_remaining != 0 {
                            cp[*cpi as usize] = UNICODE_INVALID;
                            *cpi += 1;
                        }
                        self.this_cp = c_int::from(c & 0x03);
                        self.bytes_total = 5;
                        self.bytes_remaining = 4;
                    } else if (0xfc..0xfe).contains(&c) {
                        if self.bytes_remaining != 0 {
                            cp[*cpi as usize] = UNICODE_INVALID;
                            *cpi += 1;
                        }
                        self.this_cp = c_int::from(c & 0x01);
                        self.bytes_total = 6;
                        self.bytes_remaining = 5;
                    } else {
                        cp[*cpi as usize] = UNICODE_INVALID;
                        *cpi += 1;
                    }

                    *pos += 1;
                }
            }
            VTermEncoding::UsAscii => {
                if *pos >= bytelen {
                    return;
                }
                let is_gr = bytes[*pos] & 0x80;
                while *pos < bytelen && *cpi < cplen {
                    let c = bytes[*pos] ^ is_gr;
                    if !(0x20..0x7f).contains(&c) {
                        return;
                    }
                    cp[*cpi as usize] = u32::from(c);
                    *cpi += 1;
                    *pos += 1;
                }
            }
            VTermEncoding::DecDrawing => {
                if *pos >= bytelen {
                    return;
                }
                let is_gr = bytes[*pos] & 0x80;
                while *pos < bytelen && *cpi < cplen {
                    let c = bytes[*pos] ^ is_gr;
                    if !(0x20..0x7f).contains(&c) {
                        return;
                    }
                    let mapped = DEC_DRAWING_CHARS[c as usize];
                    cp[*cpi as usize] = if mapped != 0 { mapped } else { u32::from(c) };
                    *cpi += 1;
                    *pos += 1;
                }
            }
        }
    }
}

/// Look up a `VTermEncoding` by `(type, designation)` (`vterm_lookup_encoding`).
pub fn vterm_lookup_encoding(
    enc_type: VTermEncodingType,
    designation: c_char,
) -> Option<VTermEncoding> {
    match (enc_type, designation as u8) {
        (VTermEncodingType::ENC_UTF8, b'u') => Some(VTermEncoding::Utf8),
        (VTermEncodingType::ENC_SINGLE_94, b'0') => Some(VTermEncoding::DecDrawing),
        (VTermEncodingType::ENC_SINGLE_94, b'B') => Some(VTermEncoding::UsAscii),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_and_decode_utf8_and_dec_drawing() {
        let utf8 = vterm_lookup_encoding(VTermEncodingType::ENC_UTF8, b'u' as c_char).unwrap();
        let dec = vterm_lookup_encoding(VTermEncodingType::ENC_SINGLE_94, b'0' as c_char).unwrap();
        let ascii =
            vterm_lookup_encoding(VTermEncodingType::ENC_SINGLE_94, b'B' as c_char).unwrap();
        assert_eq!(utf8, VTermEncoding::Utf8);
        assert_eq!(dec, VTermEncoding::DecDrawing);
        assert_eq!(ascii, VTermEncoding::UsAscii);
        assert!(vterm_lookup_encoding(VTermEncodingType::ENC_UTF8, b'X' as c_char).is_none());

        let mut inst = VTermEncodingInstance::default();
        inst.init(utf8);
        let input = "Aé€𐍈";
        let mut cp = [0u32; 8];
        let mut cpi: c_int = 0;
        let mut pos: usize = 0;
        inst.decode(&mut cp, &mut cpi, input.as_bytes(), &mut pos);
        assert_eq!(pos, input.len());
        assert_eq!(cpi, 4);
        assert_eq!(&cp[..4], &[0x41, 0x00E9, 0x20AC, 0x10348]);

        // DEC Special Graphics 'q' (0x71) -> 0x2500 BOX DRAWINGS LIGHT HORIZONTAL
        let mut dec_inst = VTermEncodingInstance::default();
        dec_inst.init(dec);
        let dec_in = b"Aq";
        let mut dec_cp = [0u32; 4];
        let mut dec_cpi: c_int = 0;
        let mut dec_pos: usize = 0;
        dec_inst.decode(&mut dec_cp, &mut dec_cpi, dec_in, &mut dec_pos);
        assert_eq!(dec_cpi, 2);
        assert_eq!(&dec_cp[..2], &[0x41, 0x2500]);
    }
}
