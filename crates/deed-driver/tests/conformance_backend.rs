//! Every shape the conformance suite accepts, the backend compiles.
//!
//! `crates/deed-driver/tests/corpus_backend.rs` already says the backend
//! refuses nothing in `examples/`, and that turned out to be a claim about the
//! corpus rather than about the language: a corpus is the shapes one author
//! happened to write. Two that nobody had written, a pattern inside a record
//! pattern and a `let` that takes a value apart, checked cleanly, ran under the
//! interpreter, and could not be lowered at all. Nothing said so, because a
//! test skipped for want of a compiled body looks exactly like a file with no
//! tests in it.
//!
//! `conformance/` is the other corpus, and it is written the other way round:
//! its cases exist to cover the language rather than to be a program. So this
//! holds every case the suite expects to check or to run to being lowerable,
//! which is the question `corpus_backend.rs` asks of `examples/`.
//!
//! Lowering rather than answering. What the two engines answer is
//! `agreement.rs`, which needs a program written to be compared; this needs
//! only that the backend has something to say at all.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use deed_diagnostics::SourceMap;
use deed_driver::{check_all, shipped_for, shipped_source};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels under the repository root")
        .to_path_buf()
}

/// Every case whose program is meant to be accepted, by directory name.
///
/// A `reject` case is a program the checker turns away, so the backend never
/// sees one and holding it to anything would be holding it to a program that
/// does not exist.
fn accepted() -> Vec<(String, String)> {
    let cases = root().join("conformance").join("cases");
    let mut names: BTreeSet<PathBuf> = BTreeSet::new();
    for entry in std::fs::read_dir(&cases).expect("conformance/cases should be there") {
        let path = entry.expect("a readable entry").path();
        if path.is_dir() {
            names.insert(path);
        }
    }

    let mut found = Vec::new();
    for dir in names {
        let name = dir
            .file_name()
            .expect("a directory has a name")
            .to_string_lossy()
            .to_string();
        if name.starts_with("reject-") {
            continue;
        }
        let program = dir.join("program.deed");
        if let Ok(text) = std::fs::read_to_string(&program) {
            found.push((name, text));
        }
    }

    assert!(
        found.len() > 10,
        "the suite should carry more than {} cases the checker accepts",
        found.len()
    );
    found
}

#[test]
fn the_backend_lowers_every_conformance_case_the_checker_accepts() {
    let mut refused = Vec::new();

    for (name, text) in accepted() {
        let mut sources = SourceMap::new();
        let subject = sources.add(format!("{name}/program.deed"), text.clone());
        let mut ids = vec![subject];
        for module in shipped_for([text.as_str()]) {
            let source = shipped_source(module).expect("a module that ships has a source");
            ids.push(sources.add(format!("{module}.deed"), source.to_string()));
        }

        let checks = check_all(&sources, &ids);
        assert!(
            !checks[0].has_errors(),
            "`{name}` is an accept case and should check"
        );

        let alongside: Vec<deed_mir::Alongside<'_>> = checks[1..]
            .iter()
            .map(|checked| deed_mir::Alongside {
                module: &checked.module,
                resolutions: &checked.resolutions,
                types: &checked.types,
            })
            .collect();

        if let Err(why) = deed_mir::lower_with_tests_alongside(
            &checks[0].module,
            &checks[0].resolutions,
            &checks[0].types,
            &alongside,
        ) {
            refused.push(format!("{name}: {why}"));
        }
    }

    assert!(
        refused.is_empty(),
        "the backend compiles every shape the conformance suite accepts, and now refuses \
         {}:\n{}",
        refused.len(),
        refused.join("\n")
    );
}
