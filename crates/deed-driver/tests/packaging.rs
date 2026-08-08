//! Nothing a published crate compiles may reach outside its own package.
//!
//! A `.crate` archive is one package directory. Everything above it — the
//! other crates, `std/`, the corpus, this file — is simply absent, and the two
//! ways this workspace had already broken that rule failed differently:
//! `deed-driver` embedded `std/*.deed` through three parent directories and
//! would not have compiled, while `deed-explain` generated its pages from a
//! build script that walked the workspace and would have compiled and printed
//! nothing at all. The quiet one is why this is a test rather than a habit.
//!
//! Both were found with `cargo package --list`, which is the honest check and
//! is slow enough that it belongs in the publish gate rather than here. This
//! reads the source instead: a path that climbs past the package root is the
//! shape, and it is visible without building anything.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root should be two directories up")
        .to_path_buf()
}

fn packages() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(root().join("crates"))
        .expect("crates/ should be there")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("Cargo.toml").is_file())
        .collect();
    found.sort();
    assert!(found.len() > 10, "only {} packages were found", found.len());
    found
}

fn rust_files(at: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The literal paths the `include` macros in `text` name.
///
/// Only string literals. `include!(concat!(env!("OUT_DIR"), ..))` names no
/// path at all, and that one is answered by its own test in `deed-explain`,
/// where the reason it is wrong is about that crate rather than about paths.
fn included_literals(text: &str) -> Vec<String> {
    // Comment lines are dropped first: this file's own prose names the two
    // spellings it is about, and a rule that fires on a sentence describing it
    // is a rule nobody can write about.
    let code: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut found = Vec::new();
    for macro_name in ["include_str!(", "include_bytes!(", "include!("] {
        let mut rest = code.as_str();
        while let Some(at) = rest.find(macro_name) {
            rest = &rest[at + macro_name.len()..];
            let argument = rest.split(')').next().unwrap_or("").trim();
            if let Some(path) = argument
                .strip_prefix('"')
                .and_then(|value| value.split('"').next())
            {
                found.push(path.to_string());
            }
        }
    }
    found
}

/// How far above its own directory a relative path climbs.
fn climbs(path: &str) -> usize {
    let mut depth: i32 = 0;
    let mut worst: i32 = 0;
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                depth -= 1;
                worst = worst.max(-depth);
            }
            _ => depth += 1,
        }
    }
    worst.max(0) as usize
}

#[test]
fn nothing_a_package_compiles_reads_above_its_own_root() {
    let mut checked = 0;
    for package in packages() {
        let mut files = Vec::new();
        rust_files(&package.join("src"), &mut files);

        for file in files {
            let text = std::fs::read_to_string(&file).expect("a file the walk found");
            let inside = file
                .parent()
                .expect("a file has a directory")
                .strip_prefix(&package)
                .expect("under the package")
                .components()
                .count();

            for path in included_literals(&text) {
                checked += 1;
                assert!(
                    climbs(&path) <= inside,
                    "{} includes `{path}`, which is above `{}` and would not be in its archive",
                    file.display(),
                    package.display()
                );
            }
        }
    }

    // A rule about a population of zero is not a rule. Both crates that hold
    // generated data reach one directory up out of `src/`, so this is never
    // legitimately empty.
    assert!(checked > 1, "only {checked} include paths were looked at");
}

#[test]
fn a_build_script_does_not_read_above_its_own_root() {
    for package in packages() {
        let script = package.join("build.rs");
        let Ok(text) = std::fs::read_to_string(&script) else {
            continue;
        };
        assert!(
            !text.contains(".."),
            "{} walks out of its own package, and a published crate has nothing out there",
            script.display()
        );
    }
}

#[test]
fn the_climb_is_counted_from_where_the_file_sits() {
    assert_eq!(climbs("pages.rs"), 0);
    assert_eq!(climbs("../generated/pages.rs"), 1);
    assert_eq!(climbs("../../../std/list.deed"), 3);
    // Down and back up again never leaves.
    assert_eq!(climbs("a/../b.rs"), 0);
}
