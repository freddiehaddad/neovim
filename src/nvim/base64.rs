//! Base64 encoding and decoding (`src/nvim/base64.c`).

use std::ffi::{c_char, c_void};

use crate::memory::{xfree, xmalloc};

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

/// Encode a byte buffer using Base64 (`base64_encode`).
///
/// Returns a NUL-terminated buffer allocated with `xmalloc`.
///
/// # Safety
/// `src` must be non-null and valid for `src_len` bytes when `src_len > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn base64_encode(src: *const c_char, src_len: usize) -> *mut c_char {
    assert!(!src.is_null());
    let src_slice = if src_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(src.cast::<u8>(), src_len) }
    };

    let encoded = encode_bytes(src_slice);
    let out_len = encoded.len();
    let dest = unsafe { xmalloc(out_len + 1).cast::<u8>() };
    unsafe {
        if out_len > 0 {
            std::ptr::copy_nonoverlapping(encoded.as_ptr(), dest, out_len);
        }
        *dest.add(out_len) = 0;
    }
    dest.cast::<c_char>()
}

/// Decode a Base64-encoded buffer (`base64_decode`).
///
/// The returned buffer is NOT NUL-terminated and is allocated with `xmalloc`.
/// On invalid input, sets `*out_lenp = 0` and returns null.
///
/// # Safety
/// `src` must be non-null and valid for `src_len` bytes when `src_len > 0`.
/// `out_lenp` must be a valid writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn base64_decode(
    src: *const c_char,
    src_len: usize,
    out_lenp: *mut usize,
) -> *mut c_char {
    assert!(!src.is_null());
    assert!(!out_lenp.is_null());

    let src_slice = if src_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(src.cast::<u8>(), src_len) }
    };

    let Some(decoded) = decode_bytes(src_slice) else {
        unsafe {
            *out_lenp = 0;
        }
        return std::ptr::null_mut();
    };

    let out_len = decoded.len();
    let dest = unsafe { xmalloc(out_len).cast::<u8>() };
    unsafe {
        if out_len > 0 {
            std::ptr::copy_nonoverlapping(decoded.as_ptr(), dest, out_len);
        }
        *out_lenp = out_len;
    }
    dest.cast::<c_char>()
}

#[allow(dead_code)]
fn _keep_xfree_reference() {
    let _ = xfree as unsafe fn(*mut c_void);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn rfc4648_vectors() {
        let vectors: &[(&[u8], &str)] = &[
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
            (
                b"The quick brown fox jumps over the lazy dog\n",
                "VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZwo=",
            ),
        ];

        for &(input, expected) in vectors {
            unsafe {
                let enc_ptr = base64_encode(input.as_ptr().cast::<c_char>(), input.len());
                assert!(!enc_ptr.is_null());
                let enc_str = CStr::from_ptr(enc_ptr).to_str().unwrap();
                assert_eq!(enc_str, expected);

                let mut out_len = usize::MAX;
                let dec_ptr = base64_decode(
                    expected.as_ptr().cast::<c_char>(),
                    expected.len(),
                    &mut out_len,
                );
                assert!(!dec_ptr.is_null());
                assert_eq!(out_len, input.len());
                let dec_slice = std::slice::from_raw_parts(dec_ptr.cast::<u8>(), out_len);
                assert_eq!(dec_slice, input);

                xfree(enc_ptr.cast::<c_void>());
                xfree(dec_ptr.cast::<c_void>());
            }
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
            unsafe {
                let mut out_len = 123usize;
                let ptr = base64_decode(s.as_ptr().cast::<c_char>(), s.len(), &mut out_len);
                assert!(ptr.is_null(), "expected null for invalid input {s:?}");
                assert_eq!(out_len, 0);
            }
        }
    }
}
