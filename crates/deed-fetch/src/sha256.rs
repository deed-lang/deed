//! SHA-256 over arbitrary byte slices.
//!
//! This is the NIST FIPS 180-4 algorithm, written here without any external
//! crate. The workspace carries no dependencies and a cryptographic hash for
//! content addressing is too small to import.
//!
//! The output is 32 bytes. To compare against a user-supplied hex digest,
//! encode those 32 bytes with [`hex`].

fn xor3(first: u32, second: u32, third: u32) -> u32 {
    first ^ second ^ third
}

/// Computes the SHA-256 digest of `input` and returns the 32-byte result.
pub fn sha256(input: &[u8]) -> [u8; 32] {
    // Initial hash values: the first 32 bits of the fractional parts of the
    // square roots of the first eight primes.
    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    // Round constants: the first 32 bits of the fractional parts of the cube
    // roots of the first 64 primes.
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    // Pre-processing: pad the message to a multiple of 512 bits.
    //
    // Append the bit '1', then zeros, then the 64-bit big-endian bit-length.
    // The message length in bits always fits in u64 for inputs we will ever
    // see (the compiler refuses source files that would overflow).
    let bit_len: u64 = (input.len() as u64).wrapping_mul(8);
    let mut padded: Vec<u8> = input.to_vec();
    padded.push(0x80);
    // Pad with zeros until the length in bytes is 56 mod 64 (eight bytes short
    // of a full block), so the 8-byte length fits at the end of the block.
    let remainder = padded.len() & 63;
    let zeroes = if remainder <= 56 {
        56 - remainder
    } else {
        120 - remainder
    };
    padded.resize(padded.len() + zeroes, 0);
    padded.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 64-byte (512-bit) block.
    for block in padded.chunks_exact(64) {
        // Prepare the message schedule.
        let mut w = [0u32; 64];
        for (i, chunk) in block.chunks_exact(4).enumerate().take(16) {
            w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..64 {
            let s0 = xor3(
                w[i - 15].rotate_right(7),
                w[i - 15].rotate_right(18),
                w[i - 15] >> 3,
            );
            let s1 = xor3(
                w[i - 2].rotate_right(17),
                w[i - 2].rotate_right(19),
                w[i - 2] >> 10,
            );
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        // Initialize the working variables.
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;

        // 64 rounds.
        for i in 0..64 {
            let s1 = xor3(e.rotate_right(6), e.rotate_right(11), e.rotate_right(25));
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = xor3(a.rotate_right(2), a.rotate_right(13), a.rotate_right(22));
            let maj = xor3(a & b, a & c, b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        // Add the compressed chunk to the current hash value.
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    // Produce the final hash value as big-endian bytes.
    let mut digest = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        digest[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// Encodes `bytes` as lowercase hexadecimal.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_cancels_overlapping_bits_in_each_position() {
        assert_eq!(xor3(0b11, 0b10, 0b01), 0);
    }

    /// The empty-string SHA-256 is a well-known constant.
    #[test]
    fn sha256_of_empty() {
        let digest = sha256(b"");
        assert_eq!(
            hex(&digest),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// NIST test vector: "abc".
    #[test]
    fn sha256_of_abc() {
        let digest = sha256(b"abc");
        assert_eq!(
            hex(&digest),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// NIST test vector: exactly one full block minus the padding.
    #[test]
    fn sha256_of_448_bit_message() {
        let msg = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        let digest = sha256(msg);
        assert_eq!(
            hex(&digest),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn sha256_padding_boundary_uses_one_block_then_two() {
        assert_eq!(
            hex(&sha256(&[b'a'; 55])),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
        assert_eq!(
            hex(&sha256(&[b'a'; 56])),
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
        );
    }

    /// Two different byte slices must produce different digests.
    #[test]
    fn sha256_distinguishes_inputs() {
        assert_ne!(sha256(b"left"), sha256(b"right"));
    }

    /// A single bit flip changes the digest.
    #[test]
    fn sha256_detects_a_single_bit_flip() {
        let original = b"dependency bytes";
        let mut flipped = *original;
        flipped[0] ^= 1;
        assert_ne!(sha256(original), sha256(&flipped));
    }
}
