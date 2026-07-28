//! Every diagnostic code this compiler can produce is named by some test.
//!
//! A diagnostic is not a return value. Its whole job is to be read by a
//! person, and the only way to find out whether it reads well is to have
//! looked at it. A code with no test is a message nobody has ever seen: it
//! may be unreachable, it may name the wrong span, it may say something that
//! was true two refactors ago. `DEED4005` was exactly that, sitting in the
//! type checker across sixty-six tested neighbours.
//!
//! So this is a ratchet rather than a one-off repair. Adding a code without a
//! test that mentions it fails here, and the failure names the code.
//!
//! # What this does not hold
//!
//! One test per code, and nothing finer. It matches on the constant's name and
//! on the number, so a code with several messages behind it is satisfied by a
//! test that names either, whether or not any of those messages is ever
//! rendered, and the rest are as unread as a code with no test at all.
//! `DEED5004` was that: four messages, one of them ever rendered by a test,
//! and a comment in the effects tests asserting that the tested one was the
//! only shape left.
//!
//! So if you put a second message under an existing code, nothing here will
//! ask you for a test. Write one anyway, next to the message, and have it read
//! the words rather than the code.
//!
//! Counting messages per code instead is not the fix and is deliberately not
//! attempted. A message is a `format!` in a branch, and a branch can choose
//! between two of them on a flag, so there is no set for the source to
//! enumerate; what could be counted is call sites, which is a number to keep
//! up to date rather than a claim about anything being read.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The repository root, from this crate's manifest.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn crates() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(root().join("crates"))
        .expect("crates/ should be there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no crates found under crates/");
    found
}

/// Every `pub const NAME: &str = "DEEDnnnn"`, as the pair a test could name.
///
/// Read out of the source rather than listed here, or this would be a third
/// copy of the set and the one most likely to be forgotten.
fn declared() -> Vec<(String, String)> {
    let mut codes = Vec::new();
    for krate in crates() {
        let path = krate.join("src").join("codes.rs");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("pub const ") else {
                continue;
            };
            let Some((name, rest)) = rest.split_once(':') else {
                continue;
            };
            let Some(start) = rest.find("\"DEED") else {
                continue;
            };
            let Some(end) = rest[start + 1..].find('"') else {
                continue;
            };
            codes.push((
                name.trim().to_string(),
                rest[start + 1..start + 1 + end].to_string(),
            ));
        }
    }
    codes
}

/// Everything under any crate's `tests/`, and the example programs.
///
/// The examples count because a warning the corpus produces is a warning
/// somebody reads on every run of `deed check examples`.
fn test_sources() -> String {
    let mut text = String::new();
    for krate in crates() {
        collect(&krate.join("tests"), &mut text);
    }
    collect(&root().join("examples"), &mut text);
    text
}

fn collect(dir: &Path, out: &mut String) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        // This file names no codes and would not satisfy anything, but
        // excluding it says so rather than leaving it to be noticed.
        } else if path.file_name().is_some_and(|name| name != "codes.rs")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            out.push_str(&text);
        }
    }
}

#[test]
fn every_diagnostic_code_is_named_by_a_test() {
    let declared = declared();
    // Without this the whole test passes on a parser that stopped matching,
    // which is the shape a rename of the constants would take.
    assert!(
        declared.len() > 50,
        "only {} codes found, so the source was not read properly",
        declared.len()
    );

    let sources = test_sources();
    assert!(!sources.is_empty(), "no test sources were read");

    let untested: BTreeSet<&str> = declared
        .iter()
        .filter(|(name, code)| !sources.contains(name.as_str()) && !sources.contains(code.as_str()))
        .map(|(_, code)| code.as_str())
        .collect();

    assert!(
        untested.is_empty(),
        "these diagnostics have no test, so nobody has read what they say: {}",
        untested.into_iter().collect::<Vec<_>>().join(", ")
    );
}
