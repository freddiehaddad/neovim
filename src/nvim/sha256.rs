//! FIPS-180-2 compliant SHA-256 implementation.
//!
//! Ported from `src/nvim/sha256.c` and `src/nvim/sha256.h`.

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
pub struct ContextSha256 {
    pub total: [u32; 2],
    pub state: [u32; 8],
    pub buffer: [u8; SHA256_BUFFER_SIZE],
}

impl Default for ContextSha256 {
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

impl ContextSha256 {
    #[inline]
    pub const fn new() -> Self {
        Self {
            total: [0, 0],
            state: INITIAL_STATE,
            buffer: [0; SHA256_BUFFER_SIZE],
        }
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
    let mut ctx = ContextSha256::new();
    ctx.update(buf);
    if let Some(salt_bytes) = salt {
        ctx.update(salt_bytes);
    }
    let digest = ctx.finish();
    digest_to_hex(&digest)
}

#[cfg(test)]
const SELF_TEST_MSG: [&[u8]; 2] = [
    b"abc",
    b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
];

#[cfg(test)]
const SELF_TEST_VECTOR: [&[u8; SHA256_BUFFER_SIZE]; 3] = [
    b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    b"248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
    b"cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
];

/// Perform the FIPS-180-2 self-test (`sha256_self_test`).
#[cfg(test)]
pub fn sha256_self_test() -> bool {
    for i in 0..3 {
        let output: [u8; SHA256_BUFFER_SIZE] = if i < 2 {
            sha256_hex(SELF_TEST_MSG[i], None)
        } else {
            let mut ctx = ContextSha256::new();
            let buf = [b'a'; 1000];
            for _ in 0..1000 {
                ctx.update(&buf);
            }
            digest_to_hex(&ctx.finish())
        };

        if &output != SELF_TEST_VECTOR[i] {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_layout_matches_c() {
        assert_eq!(std::mem::size_of::<ContextSha256>(), 8 + 32 + 64);
        assert_eq!(std::mem::align_of::<ContextSha256>(), 4);
    }

    #[test]
    fn fips_180_2_self_test_passes() {
        assert!(sha256_self_test());
    }

    #[test]
    fn test_vectors_match_legacy_090_sha256_spec() {
        let cases: &[(&[u8], &[u8; SHA256_BUFFER_SIZE])] = &[
            (
                b"",
                b"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"a",
                b"ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb",
            ),
            (
                b"abc",
                b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"foo\nbar",
                b"807eff6267f3f926a21d234f7b0cf867a86f47e07a532f15e8cc39ed110ca776",
            ),
            (
                b"\xde\xad\xbe\xef",
                b"5f78c33274e43fa9de5659265c1d917e25c03722dcb0b8d27db8d5feaa813953",
            ),
        ];

        for &(input, expected) in cases {
            assert_eq!(&sha256_hex(input, None), expected);
        }
    }

    #[test]
    fn test_salt_and_chunked_streaming_equivalence() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let salt = b":neovim-salt-value-1234567890";

        let combined: Vec<u8> = [data.as_slice(), salt.as_slice()].concat();
        let expected_hex = sha256_hex(&combined, None);
        assert_eq!(sha256_hex(data, Some(salt)), expected_hex);

        for chunk_size in [1, 2, 3, 7, 13, 31, 55, 56, 63, 64, 65] {
            let mut ctx = ContextSha256::new();
            for chunk in combined.chunks(chunk_size) {
                ctx.update(chunk);
            }
            let mut digest = [0u8; SHA256_SUM_SIZE];
            ctx.finish_into(&mut digest);
            assert_eq!(digest_to_hex(&digest), expected_hex);
        }
    }

    #[test]
    fn test_padding_boundary_lengths() {
        for len in [0usize, 1, 54, 55, 56, 57, 63, 64, 65, 119, 120, 121, 128] {
            let input: Vec<u8> = (0..len).map(|i| (i & 0xFF) as u8).collect();
            let one_shot = sha256_hex(&input, None);

            let mut ctx = ContextSha256::new();
            let mid = len / 2;
            ctx.update(&input[..mid]);
            ctx.update(&[]);
            ctx.update(&input[mid..]);
            assert_eq!(digest_to_hex(&ctx.finish()), one_shot);
        }
    }
}
