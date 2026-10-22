//! Base64 encoding and decoding (`src/nvim/base64.c`).

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 1-based lookup table (0 indicates a byte not in the Base64 alphabet).
const CHAR_TO_INDEX: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 64 {
        table[ALPHABET[i] as usize] = (i as u8) + 1;
        i += 1;
    }
    table
};

/// Encode `src` into Base64 bytes (without trailing NUL).
pub fn encode_bytes(src: &[u8]) -> Vec<u8> {
    let out_len = src.len().div_ceil(3) * 4;
    let mut dest = Vec::with_capacity(out_len);

    let (chunks, rem) = src.as_chunks::<3>();
    for &[b0, b1, b2] in chunks {
        dest.push(ALPHABET[(b0 >> 2) as usize]);
        dest.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]);
        dest.push(ALPHABET[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize]);
        dest.push(ALPHABET[(b2 & 0x3F) as usize]);
    }

    match *rem {
        [b0, b1] => {
            dest.push(ALPHABET[(b0 >> 2) as usize]);
            dest.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]);
            dest.push(ALPHABET[((b1 & 0x0F) << 2) as usize]);
            dest.push(b'=');
        }
        [b0] => {
            dest.push(ALPHABET[(b0 >> 2) as usize]);
            dest.push(ALPHABET[((b0 & 0x03) << 4) as usize]);
            dest.push(b'=');
            dest.push(b'=');
        }
        [] => {}
        _ => unreachable!(),
    }

    dest
}

/// Decode Base64 `src` into raw bytes, returning `None` if `src` is invalid.
pub fn decode_bytes(src: &[u8]) -> Option<Vec<u8>> {
    if !src.len().is_multiple_of(4) {
        return None;
    }

    let mut out_len = (src.len() / 4) * 3;
    if !src.is_empty() && src[src.len() - 1] == b'=' {
        out_len -= 1;
    }
    if src.len() >= 2 && src[src.len() - 2] == b'=' {
        out_len -= 1;
    }

    let mut dest = Vec::with_capacity(out_len);
    let mut acc: u32 = 0;
    let mut acc_len: u32 = 0;
    let mut leftover_i: Option<usize> = None;

    for (src_i, &c) in src.iter().enumerate() {
        let d = CHAR_TO_INDEX[c as usize];
        if d == 0 {
            if c == b'=' {
                leftover_i = Some(src_i);
                break;
            }
            return None;
        }

        acc = ((acc << 6) & 0xFFF) + u32::from(d - 1);
        acc_len += 6;
        if acc_len >= 8 {
            acc_len -= 8;
            dest.push((acc >> acc_len) as u8);
        }
    }

    if acc_len > 4 || (acc & ((1 << acc_len) - 1)) != 0 {
        return None;
    }

    if let Some(start) = leftover_i {
        let padding_len = (acc_len / 2) as usize;
        let mut padding_chars = 0usize;
        for &c in &src[start..] {
            if c != b'=' {
                return None;
            }
            padding_chars += 1;
        }
        if padding_chars != padding_len {
            return None;
        }
    }

    debug_assert_eq!(dest.len(), out_len);
    Some(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        let vectors: &[(&[u8], &[u8])] = &[
            (b"", b""),
            (b"f", b"Zg=="),
            (b"fo", b"Zm8="),
            (b"foo", b"Zm9v"),
            (b"foob", b"Zm9vYg=="),
            (b"fooba", b"Zm9vYmE="),
            (b"foobar", b"Zm9vYmFy"),
            (b"Hello\0world", b"SGVsbG8Ad29ybGQ="),
            (
                b"The quick brown fox jumps over the lazy dog\n",
                b"VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZwo=",
            ),
        ];

        for &(input, expected) in vectors {
            let enc = encode_bytes(input);
            assert_eq!(enc.as_slice(), expected);
            let dec = decode_bytes(expected).unwrap();
            assert_eq!(dec.as_slice(), input);
        }
    }

    #[test]
    fn embedded_nuls_and_all_byte_lengths() {
        let with_nul = b"Hello\0world\0\xff\xfe";
        let encoded = encode_bytes(with_nul);
        assert_eq!(decode_bytes(&encoded).as_deref(), Some(with_nul.as_slice()));

        for len in 0..=64 {
            let data: Vec<u8> = (0..len).map(|i| ((i * 73 + 19) & 0xFF) as u8).collect();
            let enc = encode_bytes(&data);
            assert_eq!(decode_bytes(&enc), Some(data));
        }
    }

    #[test]
    fn rejects_invalid_inputs() {
        let invalid = [
            "A",
            "AA",
            "AAA",
            "A..A",
            "AA=A",
            "AA/=",
            "A/==",
            "A===",
            "====",
            "Zm9vYmFyZm9vYmFyA..A",
            "Zm9vYmFyZm9vYmFyAA=A",
            "Zm9vYmFyZm9vYmFyAA/=",
            "Zm9vYmFyZm9vYmFyA/==",
            "Zm9vYmFyZm9vYmFyA===",
            "A..AZm9vYmFyZm9vYmFy",
            "Zm9vYmFyZm9vAA=A",
            "Zm9vYmFyZm9vAA/=",
            "Zm9vYmFyZm9vA/==",
            "Zm9vYmFyZm9vA===",
        ];

        for s in invalid {
            assert_eq!(
                decode_bytes(s.as_bytes()),
                None,
                "expected None for invalid input {s:?}"
            );
        }
    }
}
