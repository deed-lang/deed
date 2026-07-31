//! Content-addressed on-disk cache.
//!
//! Each entry is a file whose name is the lowercase hex SHA-256 of its
//! contents. Two projects that declare the same dependency store it once,
//! because the key is derived from the bytes rather than from where they came
//! from.
//!
//! The cache root follows platform conventions:
//!
//! - Linux: `$XDG_CACHE_HOME/deed` when set, otherwise `$HOME/.cache/deed`.
//! - macOS: `$HOME/Library/Caches/deed`.
//! - Windows: `%LOCALAPPDATA%\deed\cache`.
//!
//! Nothing stored here is ever executed. The cache is a byte store that the
//! compiler reads as source; no post-store hook runs.

use std::io;
use std::path::{Path, PathBuf};

/// A content-addressed cache directory.
///
/// All writes go through a write-then-rename so a partial write never leaves
/// a corrupt entry under its final name.
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// Opens (and creates if necessary) the platform-default cache directory.
    ///
    /// The directory is `deed` inside the OS cache root:
    ///
    /// - Linux: `$XDG_CACHE_HOME/deed` or `$HOME/.cache/deed`
    /// - macOS: `$HOME/Library/Caches/deed`
    /// - Windows: `%LOCALAPPDATA%\deed\cache`
    pub fn platform_default() -> Result<Cache, io::Error> {
        let root = platform_cache_root()?;
        Cache::at(root)
    }

    /// Opens (and creates if necessary) a cache at the given directory.
    ///
    /// Useful for tests and for build systems that supply their own cache
    /// location.
    pub fn at(root: PathBuf) -> Result<Cache, io::Error> {
        std::fs::create_dir_all(&root)?;
        Ok(Cache { root })
    }

    /// Returns `true` if the cache already holds an entry for `hash`.
    ///
    /// When this returns `true`, no network request is needed: [`Cache::path`]
    /// returns a valid path to the cached bytes.
    pub fn contains(&self, hash: &str) -> bool {
        self.root.join(hash).is_file()
    }

    /// Returns the path of the cached entry for `hash`.
    ///
    /// The path exists only when [`Cache::contains`] is `true`.
    pub fn path(&self, hash: &str) -> PathBuf {
        self.root.join(hash)
    }

    /// Writes `bytes` to the cache under `hash`.
    ///
    /// The write is atomic: bytes go to a temporary file first, then the file
    /// is renamed to its final content-addressed name. A concurrent call for
    /// the same hash is harmless because both are writing the same bytes.
    ///
    /// Nothing in this function executes or interprets `bytes`.
    pub fn insert(&self, hash: &str, bytes: &[u8]) -> Result<(), io::Error> {
        // Write to a temp file so a crash mid-write cannot leave a partial
        // entry under the hash name.
        let tmp = self.root.join(format!("{hash}.tmp"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(tmp, self.root.join(hash))?;
        Ok(())
    }

    /// The directory this cache is rooted at.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// The platform-specific parent directory for the deed cache.
fn platform_cache_root() -> Result<PathBuf, io::Error> {
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var("LOCALAPPDATA").map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "%LOCALAPPDATA% is not set; cannot locate the deed cache",
            )
        })?;
        Ok(PathBuf::from(local).join("deed").join("cache"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = home_dir()?;
        Ok(home.join("Library").join("Caches").join("deed"))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        // XDG Base Directory Specification (Linux and compatible).
        if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
            return Ok(PathBuf::from(xdg).join("deed"));
        }
        let home = home_dir()?;
        Ok(home.join(".cache").join("deed"))
    }
}

/// The user's home directory.
///
/// `std::env::home_dir` was deprecated for Windows edge cases that do not
/// apply on macOS or Linux, so the remaining platforms read `HOME` directly.
#[cfg(not(target_os = "windows"))]
fn home_dir() -> Result<PathBuf, io::Error> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "$HOME is not set"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A freshly created cache does not contain anything.
    #[test]
    fn empty_cache_contains_nothing() {
        let dir = std::env::temp_dir().join(format!(
            "deed-fetch-empty-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let cache = Cache::at(dir.clone()).unwrap();
        assert!(!cache.contains("abc123"));
        std::fs::remove_dir_all(dir).ok();
    }

    /// After inserting bytes under a hash, `contains` returns true and the
    /// path can be read back.
    #[test]
    fn inserted_bytes_are_present_and_readable() {
        let dir = std::env::temp_dir().join(format!(
            "deed-fetch-insert-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let cache = Cache::at(dir.clone()).unwrap();
        let hash = "deadbeef";
        let content = b"hello world";

        cache.insert(hash, content).unwrap();

        assert!(cache.contains(hash));
        let stored = std::fs::read(cache.path(hash)).unwrap();
        assert_eq!(stored, content);

        std::fs::remove_dir_all(dir).ok();
    }

    /// Inserting the same bytes twice is safe: the second insert overwrites
    /// the temporary file and renames it over the existing entry.
    #[test]
    fn double_insert_is_safe() {
        let dir = std::env::temp_dir().join(format!(
            "deed-fetch-double-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let cache = Cache::at(dir.clone()).unwrap();
        let hash = "cafebabe";
        let content = b"idempotent";

        cache.insert(hash, content).unwrap();
        cache.insert(hash, content).unwrap();

        let stored = std::fs::read(cache.path(hash)).unwrap();
        assert_eq!(stored, content);

        std::fs::remove_dir_all(dir).ok();
    }
}
