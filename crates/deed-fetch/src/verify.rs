//! Hash verification for fetched bytes.
//!
//! The single exported function [`verify`] checks that a slice of bytes
//! matches a caller-supplied SHA-256 hex digest. A mismatch is a hard error:
//! the bytes are never used, cached, or passed to the compiler.
//!
//! This is the only place a fetched payload is evaluated before being handed
//! to the compiler, and the evaluation is limited to computing and comparing a
//! digest. Nothing is parsed, executed, or spawned here.

use crate::sha256::{hex, sha256};

/// Why a verification failed.
#[derive(Debug, PartialEq, Eq)]
pub struct HashMismatch {
    /// The digest the dependency declaration said the bytes should have.
    pub expected: String,
    /// The digest the bytes actually produced.
    pub actual: String,
}

impl HashMismatch {
    /// A human-readable description suitable for a compiler diagnostic.
    pub fn message(&self) -> String {
        format!(
            "hash mismatch: expected {expected} but the fetched bytes hash to {actual}; \
             the dependency declaration or the remote content has changed",
            expected = self.expected,
            actual = self.actual,
        )
    }
}

impl std::fmt::Display for HashMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// Verifies that `bytes` hash to `expected_hex` under SHA-256.
///
/// Returns `Ok(())` when the digest matches. Returns `Err(HashMismatch)` when
/// the digest does not match; the caller must treat this as a hard error and
/// must not use the bytes.
///
/// `expected_hex` must be exactly 64 lowercase hexadecimal characters. Any
/// other length or character set is rejected as a mismatch so that a
/// truncated or malformed hash in a dependency declaration never silently
/// passes.
pub fn verify(expected_hex: &str, bytes: &[u8]) -> Result<(), HashMismatch> {
    let actual = hex(&sha256(bytes));
    if actual == expected_hex {
        Ok(())
    } else {
        Err(HashMismatch {
            expected: expected_hex.to_string(),
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::{hex, sha256};

    /// Correct hash is accepted.
    #[test]
    fn correct_hash_is_accepted() {
        let bytes = b"some dependency source";
        let expected = hex(&sha256(bytes));
        assert!(verify(&expected, bytes).is_ok());
    }

    /// A mismatch is a hard error, not a warning.
    #[test]
    fn mismatched_hash_is_an_error() {
        let bytes = b"real bytes";
        let wrong = hex(&sha256(b"different bytes"));
        let result = verify(&wrong, bytes);
        assert!(result.is_err());
    }

    /// The error carries both digests so the diagnostic can name both.
    #[test]
    fn mismatch_error_carries_both_digests() {
        let bytes = b"original";
        let tampered = b"tampered";
        let expected = hex(&sha256(bytes));
        let err = verify(&expected, tampered).unwrap_err();
        assert_eq!(err.expected, expected);
        assert_eq!(err.actual, hex(&sha256(tampered)));
    }

    /// A single byte change is detected.
    #[test]
    fn single_byte_tamper_is_detected() {
        let original = b"dependency source code";
        let expected = hex(&sha256(original));
        let mut tampered = original.to_vec();
        tampered[0] ^= 1;
        assert!(verify(&expected, &tampered).is_err());
    }

    /// An empty expected hash is rejected (length != 64).
    #[test]
    fn empty_expected_hash_is_rejected() {
        assert!(verify("", b"anything").is_err());
    }

    /// A truncated hash is rejected.
    #[test]
    fn truncated_hash_is_rejected() {
        let bytes = b"data";
        let full = hex(&sha256(bytes));
        let truncated = &full[..32];
        assert!(verify(truncated, bytes).is_err());
    }

    /// The mismatch message names both hashes.
    #[test]
    fn mismatch_message_names_both_hashes() {
        let bytes = b"data";
        let wrong = hex(&sha256(b"other"));
        let err = verify(&wrong, bytes).unwrap_err();
        let msg = err.message();
        assert!(
            msg.contains(&err.expected),
            "message should contain expected hash"
        );
        assert!(
            msg.contains(&err.actual),
            "message should contain actual hash"
        );
    }
}
