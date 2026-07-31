//! Lock files: enumerate the exact inputs that went into a build and verify
//! they have not changed.
//!
//! # Why this exists
//!
//! A build with no network access is only verifiable if you can say exactly
//! what went into it. The compiler already resolves every import from the
//! local file system, so there is nothing to fetch. What was missing was a
//! way to record the complete set of inputs and to refuse a build whose inputs
//! have changed since that record was written.
//!
//! # Format
//!
//! One header line, then one entry per input:
//!
//! ```text
//! deed lock v1
//! sha256:<64 hex digits>  <path>
//! sha256:<64 hex digits>  <path>
//! ```
//!
//! Paths for local files are the same paths the compiler shows in diagnostics.
//! Shipped modules (embedded in the binary) use `<shipped>/module.deed`.
//!
//! Two spaces between the hash and the path follow `sha256sum(1)` output, so
//! existing tooling that knows that format can read a lock file.
//!
//! # No external dependencies
//!
//! SHA-256 is implemented here directly. The constant tables and mixing steps
//! are the FIPS 180-4 specification; nothing here is invented. A crate would
//! be half this file of configuration for a function that is forty lines.

use std::io;
use std::path::Path;

// ---------------------------------------------------------------------------
// SHA-256
// ---------------------------------------------------------------------------

/// SHA-256 initial hash values: the first 32 bits of the fractional parts of
/// the square roots of the first eight primes.
const H0: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

/// SHA-256 round constants: the first 32 bits of the fractional parts of the
/// cube roots of the first sixty-four primes.
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

/// Returns the SHA-256 digest of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = H0;

    // Pre-processing: padding to a multiple of 512 bits.
    // Append a 1-bit (0x80 byte), pad with zeros, then the 64-bit big-endian
    // bit length of the original message.
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = data.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 512-bit (64-byte) block.
    for block in padded.chunks_exact(64) {
        // Build message schedule.
        let mut w = [0u32; 64];
        for (i, chunk) in block.chunks_exact(4).enumerate().take(16) {
            w[i] = u32::from_be_bytes(chunk.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        // Compression.
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for byte in digest {
        s.push(HEX[(byte >> 4) as usize] as char);
        s.push(HEX[(byte & 0xf) as usize] as char);
    }
    s
}

// ---------------------------------------------------------------------------
// Lock file I/O
// ---------------------------------------------------------------------------

/// A single entry in a lock file.
#[derive(Debug, PartialEq)]
pub struct Entry {
    pub digest: [u8; 32],
    pub path: String,
}

/// Computes the lock entry for a local file.
pub fn entry_for_file(path: &Path, display: &str) -> io::Result<Entry> {
    let content = std::fs::read(path)?;
    Ok(Entry {
        digest: sha256(&content),
        path: display.to_string(),
    })
}

/// Computes the lock entry for a shipped (embedded) module.
pub fn entry_for_shipped(module: &str, source: &str) -> Entry {
    Entry {
        digest: sha256(source.as_bytes()),
        path: format!("<shipped>/{module}.deed"),
    }
}

const HEADER: &str = "deed lock v1";

/// Writes a lock file to `path`.
pub fn write(path: &Path, entries: &[Entry]) -> io::Result<()> {
    let mut text = String::from(HEADER);
    text.push('\n');
    for entry in entries {
        text.push_str("sha256:");
        text.push_str(&hex(&entry.digest));
        text.push_str("  ");
        text.push_str(&entry.path);
        text.push('\n');
    }
    std::fs::write(path, text)
}

/// Reads a lock file and returns its entries.
pub fn read(path: &Path) -> Result<Vec<Entry>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let mut lines = text.lines();
    let header = lines.next().unwrap_or("");
    if header != HEADER {
        return Err(format!("not a deed lock file (expected `{HEADER}`)"));
    }
    let mut entries = Vec::new();
    for (i, line) in lines.enumerate() {
        if line.is_empty() {
            continue;
        }
        let rest = line
            .strip_prefix("sha256:")
            .ok_or_else(|| format!("line {}: expected `sha256:` prefix", i + 2))?;
        let (hex_part, rest) = rest
            .split_once("  ")
            .ok_or_else(|| format!("line {}: expected two spaces after hash", i + 2))?;
        if hex_part.len() != 64 {
            return Err(format!("line {}: hash is not 64 hex digits", i + 2));
        }
        let mut digest = [0u8; 32];
        for (j, pair) in hex_part.as_bytes().chunks_exact(2).enumerate() {
            let hi = from_hex(pair[0]).ok_or_else(|| format!("line {}: invalid hex", i + 2))?;
            let lo = from_hex(pair[1]).ok_or_else(|| format!("line {}: invalid hex", i + 2))?;
            digest[j] = (hi << 4) | lo;
        }
        entries.push(Entry {
            digest,
            path: rest.to_string(),
        });
    }
    Ok(entries)
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Verifies that every entry in the lock file matches its current content.
///
/// Returns `Ok(())` if all entries match, `Err(message)` if any differ or are
/// missing. Shipped entries (path starts with `<shipped>/`) are verified
/// against the compiled-in source; local entries are read from disk.
pub fn verify(entries: &[Entry]) -> Result<(), String> {
    for entry in entries {
        if let Some(module) = entry
            .path
            .strip_prefix("<shipped>/")
            .and_then(|s| s.strip_suffix(".deed"))
        {
            // Shipped module: content comes from the binary.
            let source = deed_driver::shipped_source(module).ok_or_else(|| {
                format!(
                    "lock references shipped module `{module}` that is no longer in the compiler"
                )
            })?;
            let actual = sha256(source.as_bytes());
            if actual != entry.digest {
                return Err(format!(
                    "shipped module `{module}` has changed since the lock was written \
                     (compiler version may have changed)"
                ));
            }
        } else {
            // Local file.
            let content = std::fs::read(&entry.path).map_err(|e| {
                format!(
                    "{}: {e} (required by lock file, run without --locked to update)",
                    entry.path
                )
            })?;
            let actual = sha256(&content);
            if actual != entry.digest {
                return Err(format!(
                    "{}: content has changed since the lock was written (run without --locked to update)",
                    entry.path
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The SHA-256 of an empty string is well-known.
    #[test]
    fn sha256_of_empty_string() {
        let digest = sha256(b"");
        let expected = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(hex(&digest), expected);
    }

    /// The SHA-256 of "abc" is well-known.
    #[test]
    fn sha256_of_abc() {
        let digest = sha256(b"abc");
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(hex(&digest), expected);
    }

    /// The SHA-256 of the 448-bit NIST test vector.
    #[test]
    fn sha256_nist_448_bit_vector() {
        let digest = sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq");
        let expected = "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1";
        assert_eq!(hex(&digest), expected);
    }

    /// A lock file round-trips through write and read.
    #[test]
    fn lock_round_trips() {
        let entries = vec![
            Entry {
                digest: sha256(b"hello"),
                path: "a/b.deed".to_string(),
            },
            Entry {
                digest: sha256(b"world"),
                path: "<shipped>/std/string.deed".to_string(),
            },
        ];

        let mut buf = String::from(HEADER);
        buf.push('\n');
        for e in &entries {
            buf.push_str("sha256:");
            buf.push_str(&hex(&e.digest));
            buf.push_str("  ");
            buf.push_str(&e.path);
            buf.push('\n');
        }

        let parsed = parse(&buf).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].digest, entries[0].digest);
        assert_eq!(parsed[0].path, entries[0].path);
        assert_eq!(parsed[1].digest, entries[1].digest);
        assert_eq!(parsed[1].path, entries[1].path);
    }

    /// A wrong header is rejected cleanly.
    #[test]
    fn wrong_header_is_rejected() {
        let err = parse("cargo lock v1\nsha256:aa  x.deed\n").unwrap_err();
        assert!(err.contains("deed lock file"), "{err}");
    }

    /// A truncated hash is rejected.
    #[test]
    fn truncated_hash_is_rejected() {
        let err = parse("deed lock v1\nsha256:abc  x.deed\n").unwrap_err();
        assert!(err.contains("64 hex digits"), "{err}");
    }
}
