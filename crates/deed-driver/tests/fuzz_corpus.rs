//! Every file in the fuzz corpus runs through the check pipeline without panicking.
//!
//! The corpus lives in `fuzz/corpus/check/`. It starts as a handful of seeds
//! and grows whenever the scheduled fuzz run finds something interesting: a
//! crash input is added here automatically so the fix for it is permanent and
//! the regression test ships with the code rather than living only in a CI
//! artifact.
//!
//! This test runs on the stable toolchain as part of the regular build. It is
//! fast because it only replays known inputs; the fuzzer that discovers new
//! ones runs separately on a schedule.

use std::fs;
use std::path::PathBuf;

use deed_diagnostics::SourceMap;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz")
        .join("corpus")
        .join("check")
}

/// Run every corpus entry through the full check pipeline.
///
/// A crash is a bug. Diagnostics are not: the corpus intentionally includes
/// files with syntax errors and type mismatches, because the parser and type
/// checker are what the fuzzer exercises and errors are their normal output.
#[test]
fn every_corpus_entry_does_not_panic() {
    let dir = corpus_dir();
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .expect("fuzz/corpus/check/ should be checked in alongside this test")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();
    entries.sort_by_key(|e| e.path());

    assert!(
        !entries.is_empty(),
        "the corpus is empty; add at least one seed to fuzz/corpus/check/"
    );

    for entry in &entries {
        let bytes = fs::read(entry.path())
            .unwrap_or_else(|e| panic!("could not read {}: {e}", entry.path().display()));
        // Non-UTF-8 bytes are not fed to the compiler in the fuzz target
        // either. Skip them here for the same reason.
        let text = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_owned(),
            Err(_) => continue,
        };
        let mut sources = SourceMap::new();
        deed_driver::check_text(&mut sources, "<corpus>", text);
    }
}
