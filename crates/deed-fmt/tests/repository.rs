//! Every `.deed` file in the repository is in canonical form.
//!
//! Without this, P4 goes back to being a preference. A formatter nothing runs
//! is a formatter that drifts from the code it is supposed to describe, and
//! the drift is invisible until someone runs it and gets a thousand line diff.

use std::path::{Path, PathBuf};

use deed_diagnostics::SourceMap;
use deed_fmt::format;

#[test]
fn every_deed_file_in_the_repository_is_formatted() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();

    assert!(!files.is_empty(), "no `.deed` files found under {root:?}");

    let mut unformatted = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path).unwrap();
        let mut sources = SourceMap::new();
        let file = sources.add(path.display().to_string(), source.clone());

        match format(file, &source) {
            Ok(formatted) if formatted == source => {}
            Ok(_) => unformatted.push(path.display().to_string()),
            Err(_) => panic!("{} does not parse", path.display()),
        }
    }

    assert!(
        unformatted.is_empty(),
        "not in canonical form, run `deed fmt`:\n  {}",
        unformatted.join("\n  ")
    );
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
            // Build output and version control are not source.
            if name == "target" || name == ".git" {
                continue;
            }
            collect(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "deed") {
            out.push(path);
        }
    }
}
