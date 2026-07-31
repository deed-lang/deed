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
//! # Shared hashing
//!
//! Hashing comes from `deed-fetch`, the workspace crate that verifies fetched
//! dependency bytes. Locking and fetching therefore use one SHA-256 oracle.

use std::io;
use std::path::Path;

use deed_fetch::sha256::{hex as shared_hex, sha256 as shared_sha256};

fn digest_matches(actual: &[u8; 32], expected: &[u8; 32]) -> bool {
    actual == expected
}

/// Returns the SHA-256 digest of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    shared_sha256(data)
}

fn hex(digest: &[u8; 32]) -> String {
    shared_hex(digest)
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
            let pair =
                std::str::from_utf8(pair).map_err(|_| format!("line {}: invalid hex", i + 2))?;
            digest[j] =
                u8::from_str_radix(pair, 16).map_err(|_| format!("line {}: invalid hex", i + 2))?;
        }
        entries.push(Entry {
            digest,
            path: rest.to_string(),
        });
    }
    Ok(entries)
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
            if !digest_matches(&actual, &entry.digest) {
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
            if !digest_matches(&actual, &entry.digest) {
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

    #[test]
    fn digest_match_requires_every_byte_to_agree() {
        let exact = [7u8; 32];
        let mut changed = exact;
        changed[31] = 8;
        assert!(digest_matches(&exact, &exact));
        assert!(!digest_matches(&changed, &exact));
    }

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

    #[test]
    fn uppercase_hex_is_parsed_to_exact_bytes() {
        let parsed = parse(
            "deed lock v1\nsha256:0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF  x.deed\n",
        )
        .unwrap();
        assert_eq!(
            parsed[0].digest,
            [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67,
                0x89, 0xab, 0xcd, 0xef,
            ]
        );
    }

    #[test]
    fn verify_accepts_exact_content_and_rejects_a_change() {
        let path = std::env::temp_dir().join(format!(
            "deed-lock-verify-{}-{:?}.deed",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, b"before").unwrap();
        let entry = entry_for_file(&path, path.to_str().unwrap()).unwrap();
        assert_eq!(verify(std::slice::from_ref(&entry)), Ok(()));

        std::fs::write(&path, b"after").unwrap();
        let error = verify(&[entry]).expect_err("changed content must be refused");
        assert!(error.contains("content has changed"), "{error}");
        std::fs::remove_file(path).ok();
    }
}
