//! FIPS-180-2 compliant SHA-256 implementation.
//!
//! Ported from `src/nvim/sha256.c` and `src/nvim/sha256.h`.

use std::cell::UnsafeCell;
use std::ffi::c_char;
use std::sync::atomic::{AtomicBool, Ordering};

pub const SHA256_BUFFER_SIZE: usize = 64;
pub const SHA256_SUM_SIZE: usize = 32;

const INITIAL_STATE: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

const K: [u32; 64] = [
    0x428A_2F98,
    0x7137_4491,
    0xB5C0_FBCF,
    0xE9B5_DBA5,
    0x3956_C25B,
    0x59F1_11F1,
    0x923F_82A4,
    0xAB1C_5ED5,
    0xD807_AA98,
    0x1283_5B01,
    0x2431_85BE,
    0x550C_7DC3,
    0x72BE_5D74,
    0x80DE_B1FE,
    0x9BDC_06A7,
    0xC19B_F174,
    0xE49B_69C1,
    0xEFBE_4786,
    0x0FC1_9DC6,
    0x240C_A1CC,
    0x2DE9_2C6F,
    0x4A74_84AA,
    0x5CB0_A9DC,
    0x76F9_88DA,
    0x983E_5152,
    0xA831_C66D,
    0xB003_27C8,
    0xBF59_7FC7,
    0xC6E0_0BF3,
    0xD5A7_9147,
    0x06CA_6351,
    0x1429_2967,
    0x27B7_0A85,
    0x2E1B_2138,
    0x4D2C_6DFC,
    0x5338_0D13,
    0x650A_7354,
    0x766A_0ABB,
    0x81C2_C92E,
    0x9272_2C85,
    0xA2BF_E8A1,
    0xA81A_664B,
    0xC24B_8B70,
    0xC76C_51A3,
    0xD192_E819,
    0xD699_0624,
    0xF40E_3585,
    0x106A_A070,
    0x19A4_C116,
    0x1E37_6C08,
    0x2748_774C,
    0x34B0_BCB5,
    0x391C_0CB3,
    0x4ED8_AA4A,
    0x5B9C_CA4F,
    0x682E_6FF3,
    0x748F_82EE,
    0x78A5_636F,
    0x84C8_7814,
    0x8CC7_0208,
    0x90BE_FFFA,
    0xA450_6CEB,
    0xBEF9_A3F7,
    0xC671_78F2,
];

static SHA256_PADDING: [u8; SHA256_BUFFER_SIZE] = {
    let mut pad = [0u8; SHA256_BUFFER_SIZE];
    pad[0] = 0x80;
    pad
};

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct context_sha256_T {
    pub total: [u32; 2],
    pub state: [u32; 8],
    pub buffer: [u8; SHA256_BUFFER_SIZE],
}

impl Default for context_sha256_T {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
const fn s0(x: u32) -> u32 {
    x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3)
}

#[inline]
const fn s1(x: u32) -> u32 {
    x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10)
}

#[inline]
const fn s2(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}

#[inline]
const fn s3(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}

#[inline]
const fn f0(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | (z & (x | y))
}

#[inline]
const fn f1(x: u32, y: u32, z: u32) -> u32 {
    z ^ (x & (y ^ z))
}

impl context_sha256_T {
    #[inline]
    pub const fn new() -> Self {
        Self {
            total: [0, 0],
            state: INITIAL_STATE,
            buffer: [0; SHA256_BUFFER_SIZE],
        }
    }

    #[inline]
    pub fn start(&mut self) {
        self.total = [0, 0];
        self.state = INITIAL_STATE;
    }

    fn process_block(&mut self, data: &[u8; SHA256_BUFFER_SIZE]) {
        let mut w = [0u32; SHA256_BUFFER_SIZE];
        for (i, &chunk) in data.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(chunk);
        }
        for t in 16..SHA256_BUFFER_SIZE {
            w[t] = s1(w[t - 2])
                .wrapping_add(w[t - 7])
                .wrapping_add(s0(w[t - 15]))
                .wrapping_add(w[t - 16]);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;

        for t in 0..SHA256_BUFFER_SIZE {
            let temp1 = h
                .wrapping_add(s3(e))
                .wrapping_add(f1(e, f, g))
                .wrapping_add(K[t])
                .wrapping_add(w[t]);
            let temp2 = s2(a).wrapping_add(f0(a, b, c));
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }

    pub fn update(&mut self, mut input: &[u8]) {
        let length = input.len();
        if length == 0 {
            return;
        }

        let mut left = (self.total[0] as usize) & (SHA256_BUFFER_SIZE - 1);

        self.total[0] = self.total[0].wrapping_add(length as u32);
        if (self.total[0] as usize) < length {
            self.total[1] = self.total[1].wrapping_add(1);
        }

        let fill = SHA256_BUFFER_SIZE - left;

        if left != 0 && input.len() >= fill {
            self.buffer[left..left + fill].copy_from_slice(&input[..fill]);
            let block = self.buffer;
            self.process_block(&block);
            input = &input[fill..];
            left = 0;
        }

        while let Some((block, rest)) = input.split_first_chunk::<SHA256_BUFFER_SIZE>() {
            self.process_block(block);
            input = rest;
        }

        if !input.is_empty() {
            self.buffer[left..left + input.len()].copy_from_slice(input);
        }
    }

    pub fn finish_into(&mut self, digest: &mut [u8; SHA256_SUM_SIZE]) {
        let high = (self.total[0] >> 29) | (self.total[1] << 3);
        let low = self.total[0] << 3;

        let mut msglen = [0u8; 8];
        msglen[0..4].copy_from_slice(&high.to_be_bytes());
        msglen[4..8].copy_from_slice(&low.to_be_bytes());

        let last = (self.total[0] & 0x3F) as usize;
        let padn = if last < 56 { 56 - last } else { 120 - last };

        self.update(&SHA256_PADDING[..padn]);
        self.update(&msglen);

        for (i, word) in self.state.iter().enumerate() {
            digest[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
    }

    pub fn finish(&mut self) -> [u8; SHA256_SUM_SIZE] {
        let mut digest = [0u8; SHA256_SUM_SIZE];
        self.finish_into(&mut digest);
        digest
    }
}

#[inline]
pub fn digest_to_hex(digest: &[u8; SHA256_SUM_SIZE]) -> [u8; SHA256_BUFFER_SIZE] {
    let mut out = [0u8; SHA256_BUFFER_SIZE];
    for (j, &byte) in digest.iter().enumerate() {
        out[j * 2] = HEX_DIGITS[(byte >> 4) as usize];
        out[j * 2 + 1] = HEX_DIGITS[(byte & 0x0F) as usize];
    }
    out
}

pub fn sha256_hex(buf: &[u8], salt: Option<&[u8]>) -> [u8; SHA256_BUFFER_SIZE] {
    let mut ctx = context_sha256_T::new();
    ctx.update(buf);
    if let Some(salt_bytes) = salt {
        ctx.update(salt_bytes);
    }
    let digest = ctx.finish();
    digest_to_hex(&digest)
}

struct StaticHexBuf(UnsafeCell<[u8; SHA256_BUFFER_SIZE + 1]>);

// Matches C `static char hexit[SHA256_BUFFER_SIZE + 1]` in single-threaded Neovim main loop.
unsafe impl Sync for StaticHexBuf {}

static HEXIT: StaticHexBuf = StaticHexBuf(UnsafeCell::new([0u8; SHA256_BUFFER_SIZE + 1]));
static SELF_TESTED: AtomicBool = AtomicBool::new(false);
static SELF_TEST_FAILURES: AtomicBool = AtomicBool::new(false);

const SELF_TEST_MSG: [&[u8]; 2] = [
    b"abc",
    b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
];

const SELF_TEST_VECTOR: [&[u8; SHA256_BUFFER_SIZE]; 3] = [
    b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    b"248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
    b"cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
];

/// Initialize a SHA-256 context (`sha256_start`).
///
/// # Safety
/// `ctx` must be null or point to writable memory for `context_sha256_T`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sha256_start(ctx: *mut context_sha256_T) {
    if ctx.is_null() {
        return;
    }
    // Use `ptr::write` because C callers pass an uninitialized stack `context_sha256_T`.
    unsafe {
        ctx.write(context_sha256_T::new());
    }
}

/// Feed `input[..length]` into the SHA-256 context (`sha256_update`).
///
/// # Safety
/// `ctx` must point to an initialized `context_sha256_T`, and `input` must be valid for `length`
/// bytes when `length > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sha256_update(
    ctx: *mut context_sha256_T,
    input: *const u8,
    length: usize,
) {
    if ctx.is_null() || length == 0 || input.is_null() {
        return;
    }
    let slice = unsafe { std::slice::from_raw_parts(input, length) };
    let ctx_ref = unsafe { &mut *ctx };
    ctx_ref.update(slice);
}

/// Finalize the SHA-256 context and write the 32-byte digest (`sha256_finish`).
///
/// # Safety
/// `ctx` must point to an initialized `context_sha256_T`, and `digest` must point to at least
/// `SHA256_SUM_SIZE` (32) writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sha256_finish(ctx: *mut context_sha256_T, digest: *mut u8) {
    if ctx.is_null() || digest.is_null() {
        return;
    }
    let ctx_ref = unsafe { &mut *ctx };
    let digest_arr = unsafe { &mut *(digest as *mut [u8; SHA256_SUM_SIZE]) };
    ctx_ref.finish_into(digest_arr);
}

/// Perform the FIPS-180-2 self-test (`sha256_self_test`).
#[unsafe(no_mangle)]
pub extern "C" fn sha256_self_test() -> bool {
    if SELF_TESTED.swap(true, Ordering::Relaxed) {
        return !SELF_TEST_FAILURES.load(Ordering::Relaxed);
    }

    let mut failures = false;

    for i in 0..3 {
        let output: [u8; SHA256_BUFFER_SIZE] = if i < 2 {
            let msg = SELF_TEST_MSG[i];
            let hex_ptr = unsafe { sha256_bytes(msg.as_ptr(), msg.len(), std::ptr::null(), 0) };
            let hex_slice =
                unsafe { std::slice::from_raw_parts(hex_ptr.cast::<u8>(), SHA256_BUFFER_SIZE) };
            let mut arr = [0u8; SHA256_BUFFER_SIZE];
            arr.copy_from_slice(hex_slice);
            arr
        } else {
            let mut ctx = context_sha256_T::new();
            let buf = [b'a'; 1000];
            for _ in 0..1000 {
                ctx.update(&buf);
            }
            let sha256sum = ctx.finish();
            digest_to_hex(&sha256sum)
        };

        if &output != SELF_TEST_VECTOR[i] {
            failures = true;
        }
    }

    SELF_TEST_FAILURES.store(failures, Ordering::Relaxed);
    !failures
}

/// Gets the hex digest of `buf[..buf_len]` (and optional `salt[..salt_len]`) in a static buffer.
///
/// # Safety
/// `buf` must be valid for `buf_len` bytes when `buf_len > 0`, and if `salt` is non-null it must
/// be valid for `salt_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sha256_bytes(
    buf: *const u8,
    buf_len: usize,
    salt: *const u8,
    salt_len: usize,
) -> *const c_char {
    sha256_self_test();

    let mut ctx = context_sha256_T::new();
    if buf_len > 0 && !buf.is_null() {
        let buf_slice = unsafe { std::slice::from_raw_parts(buf, buf_len) };
        ctx.update(buf_slice);
    }
    if !salt.is_null() && salt_len > 0 {
        let salt_slice = unsafe { std::slice::from_raw_parts(salt, salt_len) };
        ctx.update(salt_slice);
    }

    let sha256sum = ctx.finish();
    let hex = digest_to_hex(&sha256sum);

    unsafe {
        let out = &mut *HEXIT.0.get();
        out[..SHA256_BUFFER_SIZE].copy_from_slice(&hex);
        out[SHA256_BUFFER_SIZE] = 0;
        out.as_ptr().cast::<c_char>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    use std::sync::Mutex;

    static HEX_BUF_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn context_layout_matches_c() {
        assert_eq!(std::mem::size_of::<context_sha256_T>(), 8 + 32 + 64);
        assert_eq!(std::mem::align_of::<context_sha256_T>(), 4);
    }

    #[test]
    fn fips_180_2_self_test_passes() {
        assert!(sha256_self_test());
        // Second call hits cached fast path.
        assert!(sha256_self_test());
    }

    #[test]
    fn test_vectors_match_legacy_090_sha256_spec() {
        let cases: &[(&[u8], &str)] = &[
            (
                b"",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"a",
                "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb",
            ),
            (
                b"abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"foo\nbar",
                "807eff6267f3f926a21d234f7b0cf867a86f47e07a532f15e8cc39ed110ca776",
            ),
            (
                b"\xde\xad\xbe\xef",
                "5f78c33274e43fa9de5659265c1d917e25c03722dcb0b8d27db8d5feaa813953",
            ),
        ];

        for &(input, expected) in cases {
            let _guard = HEX_BUF_LOCK.lock().unwrap();
            let ptr = unsafe { sha256_bytes(input.as_ptr(), input.len(), std::ptr::null(), 0) };
            let got = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn test_salt_and_chunked_streaming_equivalence() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let salt = b":neovim-salt-value-1234567890";

        let combined: Vec<u8> = [data.as_slice(), salt.as_slice()].concat();
        let expected_hex = sha256_hex(&combined, None);

        {
            let _guard = HEX_BUF_LOCK.lock().unwrap();
            let salted_ptr =
                unsafe { sha256_bytes(data.as_ptr(), data.len(), salt.as_ptr(), salt.len()) };
            let salted_str = unsafe { CStr::from_ptr(salted_ptr) }.to_bytes();
            assert_eq!(salted_str, &expected_hex);
        }

        // Verify byte-by-byte and irregular chunk updates produce identical digest.
        for chunk_size in [1, 2, 3, 7, 13, 31, 55, 56, 63, 64, 65] {
            let mut ctx = std::mem::MaybeUninit::<context_sha256_T>::uninit();
            unsafe {
                sha256_start(ctx.as_mut_ptr());
                for chunk in combined.chunks(chunk_size) {
                    sha256_update(ctx.as_mut_ptr(), chunk.as_ptr(), chunk.len());
                }
                let mut digest = [0u8; SHA256_SUM_SIZE];
                sha256_finish(ctx.as_mut_ptr(), digest.as_mut_ptr());
                assert_eq!(digest_to_hex(&digest), expected_hex);
            }
        }
    }

    #[test]
    fn test_padding_boundary_lengths() {
        // Specifically exercise last < 56, last == 55, last == 56, last == 63, last == 64.
        for len in [0usize, 1, 54, 55, 56, 57, 63, 64, 65, 119, 120, 121, 128] {
            let input: Vec<u8> = (0..len).map(|i| (i & 0xFF) as u8).collect();
            let one_shot = sha256_hex(&input, None);

            let mut ctx = context_sha256_T::new();
            let mid = len / 2;
            ctx.update(&input[..mid]);
            ctx.update(&[]);
            ctx.update(&input[mid..]);
            assert_eq!(digest_to_hex(&ctx.finish()), one_shot);
        }
    }
}
