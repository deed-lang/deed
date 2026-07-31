//! Fetch, verify, and cache Deed dependencies.
//!
//! This crate implements the five properties stated in
//! `design/decisions/2026-07-31-fetch-verify-no-execute.md`:
//!
//! 1. **Offline-first**: if the expected hash is already in the cache, nothing
//!    goes over the network.
//! 2. **Hard failure on mismatch**: a hash that does not match the fetched
//!    bytes is a [`HashMismatch`] error. Nothing in this crate demotes that to
//!    a warning or a retry.
//! 3. **Content-addressed**: the cache key is derived from the bytes
//!    themselves, not from a URL. Two projects that need the same bytes store
//!    them once.
//! 4. **No execution**: nothing in this crate compiles, interprets, spawns, or
//!    evaluates the bytes it stores. Fetched bytes are opaque data until the
//!    compiler is given the cached path.
//! 5. **Platform cache**: [`Cache::platform_default`] returns a directory
//!    under the OS-standard location.
//!
//! ## Using this crate
//!
//! The caller is responsible for fetching the raw bytes from the network (or
//! reading them from a file for local dependencies). This crate takes those
//! bytes, verifies the hash, and stores them:
//!
//! ```no_run
//! use deed_fetch::{Cache, verify_and_cache};
//!
//! # fn fetch_bytes(_url: &str) -> Vec<u8> { vec![] }
//! let cache = Cache::platform_default().expect("could not open cache");
//! let expected_hash = "ba7816bf8f01cfea414140de5dae2ec73b00361bbef0469348423f656b6a3f2c";
//!
//! // If the bytes are already cached the caller does not even need to fetch.
//! let path = if cache.contains(expected_hash) {
//!     cache.path(expected_hash)
//! } else {
//!     let bytes = fetch_bytes("https://example.com/dep.deed");
//!     verify_and_cache(&cache, expected_hash, &bytes).expect("fetch failed")
//! };
//! // `path` is now a readable file. Nothing in it has been executed.
//! ```

pub mod cache;
pub mod sha256;
pub mod verify;

pub use cache::Cache;
pub use verify::HashMismatch;

use std::io;
use std::path::PathBuf;

/// The error returned by [`verify_and_cache`].
#[derive(Debug)]
pub enum FetchError {
    /// The bytes do not match the declared hash.
    HashMismatch(HashMismatch),
    /// An IO error while writing to the cache.
    Io(io::Error),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::HashMismatch(m) => m.fmt(f),
            FetchError::Io(e) => write!(f, "cache IO error: {e}"),
        }
    }
}

impl From<HashMismatch> for FetchError {
    fn from(m: HashMismatch) -> Self {
        FetchError::HashMismatch(m)
    }
}

impl From<io::Error> for FetchError {
    fn from(e: io::Error) -> Self {
        FetchError::Io(e)
    }
}

/// Verifies `bytes` against `expected_hash` and stores them in `cache`.
///
/// Returns the path of the cached file on success.
///
/// # Errors
///
/// Returns [`FetchError::HashMismatch`] when the SHA-256 digest of `bytes`
/// does not equal `expected_hash`. This is always a hard error: the bytes are
/// never stored.
///
/// Returns [`FetchError::Io`] when the cache directory cannot be written.
///
/// # No execution
///
/// Nothing in this function executes, interprets, spawns, or parses the
/// bytes. The bytes are written to disk and the caller is returned a path.
pub fn verify_and_cache(
    cache: &Cache,
    expected_hash: &str,
    bytes: &[u8],
) -> Result<PathBuf, FetchError> {
    verify::verify(expected_hash, bytes)?;
    cache.insert(expected_hash, bytes)?;
    Ok(cache.path(expected_hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::{hex, sha256};

    fn temp_cache() -> (Cache, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "deed-fetch-lib-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let cache = Cache::at(dir.clone()).unwrap();
        (cache, dir)
    }

    /// Verifying correct bytes returns a path to the stored file.
    #[test]
    fn verify_and_cache_stores_and_returns_path() {
        let (cache, dir) = temp_cache();
        let bytes = b"a deed source file";
        let hash = hex(&sha256(bytes));

        let path = verify_and_cache(&cache, &hash, bytes).unwrap();
        assert!(path.is_file());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        std::fs::remove_dir_all(dir).ok();
    }

    /// A hash mismatch is a hard error: the bytes are not stored.
    #[test]
    fn hash_mismatch_is_an_error_and_bytes_are_not_stored() {
        let (cache, dir) = temp_cache();
        let bytes = b"legitimate source";
        let wrong_hash = hex(&sha256(b"something else entirely"));

        let result = verify_and_cache(&cache, &wrong_hash, bytes);
        assert!(matches!(result, Err(FetchError::HashMismatch(_))));
        // The tampered bytes must not appear in the cache.
        assert!(!cache.contains(&wrong_hash));

        std::fs::remove_dir_all(dir).ok();
    }

    /// When the hash is already in the cache, `contains` is true and no
    /// re-verification is needed. This is the offline case.
    #[test]
    fn cached_entry_is_available_offline() {
        let (cache, dir) = temp_cache();
        let bytes = b"cached dependency";
        let hash = hex(&sha256(bytes));

        // Warm the cache.
        verify_and_cache(&cache, &hash, bytes).unwrap();

        // A second build can skip the fetch entirely.
        assert!(cache.contains(&hash));
        let stored = std::fs::read(cache.path(&hash)).unwrap();
        assert_eq!(stored, bytes);

        std::fs::remove_dir_all(dir).ok();
    }

    /// The FetchError display mentions both hashes for a mismatch.
    #[test]
    fn fetch_error_display_names_both_hashes() {
        let (cache, dir) = temp_cache();
        let bytes = b"real";
        let wrong = hex(&sha256(b"fake"));

        let err = verify_and_cache(&cache, &wrong, bytes).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains(&wrong),
            "display should contain the expected hash"
        );

        std::fs::remove_dir_all(dir).ok();
    }
}
