//! Which files the server checks together.
//!
//! The unit of compilation in Deed is the set of files handed to the compiler,
//! and until now the server handed it one. That is defensible until you open a
//! file with a `use` in it, at which point the module it imports is not among
//! the files being compiled and the editor puts a red line under code that is
//! correct. A server that reports errors on a working program is worse than
//! one that reports nothing.
//!
//! So the set is the workspace the editor already told us about. That is not a
//! guess: `initialize` carries `workspaceFolders`, and taking what the editor
//! says is the folder is the same answer `deed check src/` gives when a person
//! says which directory they mean.
//!
//! The walk is the walk the command line tool does, skipping build output and
//! version control, and it happens on every check. That is O(workspace) per
//! keystroke, and this file used to say P9 had never been measured and that
//! this was the thing that would make measuring it interesting. It was, in
//! `crates/deed-driver/examples/edit_loop.rs`, which rechecks workspaces of 1
//! to 512 files and reports both the wall clock and the share of it spent on
//! files that did not change. It comes out linear at about 70us a file, so 512
//! files is 38ms a keystroke, inside the 100ms P9 asks for. Roughly all of that
//! is spent rechecking files nothing touched, which is where a cache would go
//! if the number ever stopped being fine.

use std::path::{Path, PathBuf};

use crate::json::Json;
use crate::uri;

/// The folders an editor said it has open.
#[derive(Default)]
pub struct Workspace {
    roots: Vec<PathBuf>,
}

impl Workspace {
    /// Reads the folders out of an `initialize` request.
    ///
    /// `workspaceFolders` first, because it is the one that can carry more
    /// than one, and `rootUri` after it for clients that only send that.
    /// Neither is required, and an editor that sends no folder at all gets the
    /// single file behaviour rather than a guess about which directory it
    /// meant.
    pub fn from_initialize(message: &Json) -> Self {
        let mut roots = Vec::new();

        if let Some(folders) = message
            .at(&["params", "workspaceFolders"])
            .and_then(Json::as_array)
        {
            for folder in folders {
                if let Some(path) = folder
                    .get("uri")
                    .and_then(Json::as_str)
                    .and_then(uri::to_path)
                {
                    roots.push(path);
                }
            }
        }

        if roots.is_empty()
            && let Some(path) = message
                .at(&["params", "rootUri"])
                .and_then(Json::as_str)
                .and_then(uri::to_path)
        {
            roots.push(path);
        }

        Self { roots }
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Every `.deed` file under every folder, sorted.
    ///
    /// Sorted so that two runs over the same tree produce the same list, which
    /// is what keeps a diagnostic from moving between files depending on the
    /// order the filesystem happened to hand things back.
    pub fn files(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for root in &self.roots {
            collect(root, &mut found);
        }
        found.sort();
        found.dedup();
        found
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if path.is_dir() {
            // Build output and version control are not source. A dot directory
            // is somebody's tooling, and walking into one is how a language
            // server ends up reading a few thousand files it was not asked
            // about.
            if name == "target" || name.starts_with('.') {
                continue;
            }
            collect(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "deed") {
            out.push(path);
        }
    }
}

/// A path in the one form two spellings of it can be compared in.
///
/// The same file arrives from two directions here, as a URI the editor sent
/// and as an entry from a directory walk, and on Windows those differ in the
/// case of the drive letter and in which slash they use. Comparing them
/// unresolved means adding one file twice, and two files claiming one `module`
/// line is an error about a program that is fine.
pub fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::Workspace;
    use crate::json::parse;

    #[test]
    fn no_folder_means_no_workspace() {
        // An editor that says nothing about folders gets the single file
        // behaviour, which is honest, rather than the working directory, which
        // would be a guess about a machine the server knows nothing about.
        let message = parse("{\"params\":{}}").unwrap();
        assert!(Workspace::from_initialize(&message).is_empty());
    }

    #[test]
    fn folders_are_read_out_of_the_request() {
        let message = parse(
            "{\"params\":{\"workspaceFolders\":[\
             {\"uri\":\"file:///work/one\",\"name\":\"one\"},\
             {\"uri\":\"file:///work/two\",\"name\":\"two\"}]}}",
        )
        .unwrap();

        let workspace = Workspace::from_initialize(&message);
        assert!(!workspace.is_empty());
    }

    #[test]
    fn root_uri_is_used_when_there_are_no_folders() {
        // The older spelling. Some clients still only send this one.
        let message = parse("{\"params\":{\"rootUri\":\"file:///work/one\"}}").unwrap();
        assert!(!Workspace::from_initialize(&message).is_empty());
    }

    #[test]
    fn a_folder_that_is_not_a_local_path_is_dropped() {
        let message =
            parse("{\"params\":{\"workspaceFolders\":[{\"uri\":\"vscode-vfs://github/a/b\"}]}}")
                .unwrap();
        assert!(Workspace::from_initialize(&message).is_empty());
    }

    #[test]
    fn the_walk_finds_deed_files_and_skips_build_output() {
        let dir = std::env::temp_dir().join(format!(
            "deed-workspace-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("src/a.deed"), "module a\n").unwrap();
        std::fs::write(dir.join("src/b.deed"), "module b\n").unwrap();
        std::fs::write(dir.join("src/notes.txt"), "not source").unwrap();
        std::fs::write(dir.join("target/old.deed"), "module old\n").unwrap();
        std::fs::write(dir.join(".git/hook.deed"), "module hook\n").unwrap();

        let workspace = Workspace {
            roots: vec![dir.clone()],
        };
        let files = workspace.files();

        assert_eq!(files.len(), 2, "{files:?}");
        assert!(files[0].ends_with("a.deed"), "{files:?}");
        assert!(files[1].ends_with("b.deed"), "{files:?}");

        std::fs::remove_dir_all(dir).ok();
    }
}
