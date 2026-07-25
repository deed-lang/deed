//! Resolving a name inside a directory capability, without letting it out.
//!
//! This is the part of the capability story that has to actually hold. A `Dir`
//! is only worth something if a function holding one rooted at `cache` has no
//! way to reach its parent, and "has no way" means every way, not the obvious
//! one.
//!
//! The rules are deliberately narrower than the filesystem allows:
//!
//! - a name is one component, so no separator of either flavour
//! - `.` and `..` are refused by name, and so is the empty string
//! - anything absolute, or carrying a drive prefix, is refused
//! - the result is canonicalized and checked to still be under the root
//!
//! The last rule is the one that matters and the one that is easy to leave
//! out. Refusing `..` textually says nothing about a symlink pointing at
//! `/etc`, and a check that can be walked around with `ln -s` is not a check.

use std::path::{Component, Path, PathBuf};

/// Why a name was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The empty string names nothing.
    Empty,
    /// More than one component, or a component that is not a plain name.
    NotOneComponent,
    /// `.` or `..`.
    Traversal,
    /// Absolute, or carrying a root or drive prefix.
    Absolute,
    /// The name resolves outside the directory. Symlinks land here.
    Outside,
    /// Nothing is there, which is not an attack, just an absence.
    Missing,
}

impl Refused {
    /// The message a program sees, which is the rule that was hit.
    pub fn message(&self, name: &str) -> String {
        match self {
            Refused::Empty => "the empty string names nothing".to_string(),
            Refused::NotOneComponent => {
                format!("`{name}` is not a single name, and a `Dir` only takes one at a time")
            }
            Refused::Traversal => {
                format!("`{name}` would leave the directory, and there is no way out of a `Dir`")
            }
            Refused::Absolute => {
                format!("`{name}` is an absolute path, and a `Dir` only names things inside itself")
            }
            Refused::Outside => {
                format!("`{name}` resolves outside the directory, so it is refused")
            }
            Refused::Missing => format!("`{name}` is not there"),
        }
    }
}

/// Checks a name without touching the filesystem.
///
/// Split out from [`resolve`] because these rules are the ones a reader has to
/// be able to verify, and mixing them with io errors makes that harder.
fn check_name(name: &str) -> Result<(), Refused> {
    if name.is_empty() {
        return Err(Refused::Empty);
    }
    if name == "." || name == ".." {
        return Err(Refused::Traversal);
    }
    // Checked textually as well as structurally. On Unix a backslash is an
    // ordinary character in a filename, so `Path` would call `a\b` one
    // component, and a name that means two different things on two platforms
    // is not something to hand to a security check.
    if name.contains('/') || name.contains('\\') {
        return Err(Refused::NotOneComponent);
    }

    let path = Path::new(name);
    let mut components = path.components();
    match components.next() {
        Some(Component::Normal(_)) => {}
        Some(Component::RootDir | Component::Prefix(_)) => return Err(Refused::Absolute),
        Some(Component::CurDir | Component::ParentDir) => return Err(Refused::Traversal),
        None => return Err(Refused::Empty),
    }
    if components.next().is_some() {
        return Err(Refused::NotOneComponent);
    }

    Ok(())
}

/// Resolves `name` inside `root`, or says which rule refused it.
///
/// `root` is expected to be canonical already, which it is, because the only
/// way to obtain a `Dir` is from this function or from the runtime, and both
/// canonicalize.
pub fn resolve(root: &Path, name: &str) -> Result<PathBuf, Refused> {
    check_name(name)?;

    let joined = root.join(name);
    let canonical = joined.canonicalize().map_err(|_| Refused::Missing)?;

    // The check that survives symlinks. Everything above is a fast refusal of
    // things that were never going to work; this is the one that decides.
    if !canonical.starts_with(root) {
        return Err(Refused::Outside);
    }

    Ok(canonical)
}

/// Canonicalizes a path so it can be used as the root of a `Dir`.
pub fn root(path: &Path) -> Result<PathBuf, Refused> {
    path.canonicalize().map_err(|_| Refused::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_is_fine() {
        assert_eq!(check_name("config.toml"), Ok(()));
        assert_eq!(check_name("cache"), Ok(()));
        assert_eq!(check_name(".hidden"), Ok(()));
    }

    #[test]
    fn the_empty_string_names_nothing() {
        assert_eq!(check_name(""), Err(Refused::Empty));
    }

    #[test]
    fn dot_and_dotdot_are_refused_by_name() {
        assert_eq!(check_name("."), Err(Refused::Traversal));
        assert_eq!(check_name(".."), Err(Refused::Traversal));
    }

    #[test]
    fn separators_of_either_flavour_are_refused() {
        // Not because a slash is dangerous but because a `Dir` narrows one
        // step at a time, and `open("a/b")` is asking to skip the middle one.
        assert_eq!(check_name("a/b"), Err(Refused::NotOneComponent));
        assert_eq!(check_name("a\\b"), Err(Refused::NotOneComponent));
        assert_eq!(check_name("../etc/passwd"), Err(Refused::NotOneComponent));
        assert_eq!(check_name("..\\..\\windows"), Err(Refused::NotOneComponent));
    }

    #[test]
    fn absolute_paths_are_refused() {
        assert_eq!(check_name("/etc/passwd"), Err(Refused::NotOneComponent));
        assert_eq!(check_name("C:\\Windows"), Err(Refused::NotOneComponent));
    }

    #[test]
    fn a_name_resolves_inside_the_root() {
        let dir = scratch("inside");
        std::fs::write(dir.join("note.txt"), "hello").unwrap();

        let root = root(&dir).unwrap();
        let resolved = resolve(&root, "note.txt").unwrap();
        assert!(resolved.starts_with(&root));
        assert_eq!(std::fs::read_to_string(resolved).unwrap(), "hello");

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn something_that_is_not_there_is_not_an_attack() {
        let dir = scratch("absent");
        let root = root(&dir).unwrap();
        assert_eq!(resolve(&root, "nope.txt"), Err(Refused::Missing));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_symlink_pointing_out_is_refused() {
        let Some((dir, outside)) = symlink_escape("symlink") else {
            // Creating a symlink needs a privilege this platform may not
            // grant. Skipping is honest; pretending it passed is not.
            return;
        };

        let root = root(&dir).unwrap();
        assert_eq!(
            resolve(&root, "escape"),
            Err(Refused::Outside),
            "a symlink walked out of the directory"
        );

        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(outside).ok();
    }

    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("vow-sandbox-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A directory containing a symlink to somewhere outside it, if the
    /// platform allows making one.
    fn symlink_escape(tag: &str) -> Option<(PathBuf, PathBuf)> {
        let dir = scratch(tag);
        let outside = scratch(&format!("{tag}-outside"));
        std::fs::write(outside.join("secret.txt"), "not yours").unwrap();

        let link = dir.join("escape");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&outside, &link).is_ok();

        if made {
            Some((dir, outside))
        } else {
            std::fs::remove_dir_all(&dir).ok();
            std::fs::remove_dir_all(&outside).ok();
            None
        }
    }
}
