//! The whole corpus, run past the backend.
//!
//! `crates/deed-driver/tests/agreement.rs` compares the two engines on
//! programs written for the purpose. This asks a different question of the
//! same machinery: over every file in `examples/` and `std/`, which
//! functions does the backend compile, and does it ever compile one into
//! something that answers differently.
//!
//! Two things are guarded. Nothing in the corpus may compile into a module
//! that fails to encode, and what the backend cannot compile has to say so
//! rather than produce one quietly. Neither is a coverage target: the
//! backend compiles a subset of the language on purpose and the number below
//! is a floor rather than a goal, so that a change which quietly stops
//! compiling half the corpus is loud.

use std::path::{Path, PathBuf};

use deed_codegen::compile;
use deed_diagnostics::SourceMap;
use deed_driver::{check_all, imports_of, shipped_modules, shipped_source};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Every `.deed` file the repository carries, corpus and library both.
fn sources() -> Vec<(String, String)> {
    let mut found = Vec::new();
    for directory in ["examples", "std"] {
        let base = root().join(directory);
        collect(&base, &mut found);
    }
    found.sort();
    assert!(
        found.len() > 20,
        "the corpus should be more than a handful of files, found {}",
        found.len()
    );
    found
}

fn collect(at: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "deed") {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a file name")
                .to_string();
            out.push((
                name,
                std::fs::read_to_string(&path).expect("a readable file"),
            ));
        }
    }
}

/// What happened to one file.
struct Outcome {
    name: String,
    /// `None` when the backend compiled it, otherwise what stopped it.
    refused: Option<String>,
}

fn walked() -> Vec<Outcome> {
    let mut outcomes = Vec::new();

    for (name, text) in sources() {
        let mut map = SourceMap::new();
        let mut ids = vec![map.add(name.clone(), text.clone())];

        // What module this file declares, so that the shipped copy of the
        // same module is not added alongside it.  A file under `std/` is the
        // canonical source of a shipped module, so handing both to `check_all`
        // would be a real collision rather than a useful test.
        let declared = imports_of(&text).map(|(module, _)| module);

        // A file that imports something needs it alongside, the same way the
        // command line hands the compiler both.
        for module in shipped_modules() {
            if declared.as_deref() == Some(module) {
                // This corpus file IS the shipped module; skip the embedded
                // copy to avoid a spurious module-path collision.
                continue;
            }
            if let Some(source) = shipped_source(module) {
                ids.push(map.add(format!("<shipped>/{module}.deed"), source.to_string()));
            }
        }

        let checks = check_all(&map, &ids);
        let subject = &checks[0];
        if subject.has_errors() {
            // A file that does not check on its own is one the corpus reaches
            // through an import, and the backend is not the thing to ask
            // about it.
            continue;
        }

        // The modules this one imports, since a call into one is lowered from
        // that module's own syntax.
        let alongside: Vec<deed_mir::Alongside<'_>> = checks[1..]
            .iter()
            .map(|checked| deed_mir::Alongside {
                module: &checked.module,
                resolutions: &checked.resolutions,
                types: &checked.types,
            })
            .collect();

        let refused = match deed_mir::lower_alongside(
            &subject.module,
            &subject.resolutions,
            &subject.types,
            &alongside,
        ) {
            Err(why) => Some(why.to_string()),
            Ok(lowered) => match compile(&lowered) {
                Err(why) => Some(why.to_string()),
                Ok(module) => {
                    let bytes = module.encode();
                    assert_eq!(
                        &bytes[..4],
                        b"\0asm",
                        "`{name}` compiled into something that is not a module"
                    );
                    None
                }
            },
        };

        outcomes.push(Outcome { name, refused });
    }

    assert!(!outcomes.is_empty(), "nothing in the corpus was looked at");
    outcomes
}

/// Whatever the backend compiles, it compiles into a module.
///
/// The interesting half is the assertion inside [`walked`]: a file that gets
/// past `compile` has to encode as something a runtime would recognise.
#[test]
fn nothing_in_the_corpus_compiles_into_something_that_is_not_a_module() {
    let outcomes = walked();
    let compiled = outcomes.iter().filter(|one| one.refused.is_none()).count();
    assert!(
        compiled > 0,
        "the backend compiled nothing at all in the corpus, which is either a\
         regression or a test that stopped looking at anything"
    );
}

/// What the backend cannot compile, it says.
///
/// The failure mode this rules out is the quiet one: a file that neither
/// compiles nor explains itself. Every refusal names a function and what it
/// found, because that is the whole of what a person gets back from
/// `deed build`.
#[test]
fn every_refusal_says_what_it_found() {
    for outcome in walked() {
        let Some(why) = outcome.refused else {
            continue;
        };
        assert!(
            why.contains("not") || why.contains("does not"),
            "`{}` was refused without saying what it found: {why}",
            outcome.name
        );
        assert!(
            why.len() > 10,
            "`{}` was refused with nothing useful in it: {why}",
            outcome.name
        );
    }
}

/// Every file in the corpus that checks on its own compiles.
///
/// This was a list of names and a floor, because the backend compiled seven
/// of the thirty-five and the interesting question was which. It is all of
/// them now (#877), so the list is the corpus and what is written down is
/// that nothing is refused.
///
/// What it catches is the same thing the list caught, from the other end: a
/// change that stops compiling something. The name of whatever stopped is in
/// the failure, along with what stopped it, which is what the list existed to
/// say.
#[test]
fn the_backend_compiles_the_whole_corpus() {
    let outcomes = walked();
    let refused: Vec<String> = outcomes
        .iter()
        .filter_map(|one| {
            let why = one.refused.as_deref()?;
            Some(format!("`{}`: {why}", one.name))
        })
        .collect();

    assert!(
        refused.is_empty(),
        "the backend compiles every file in the corpus and now refuses {}:\n{}",
        refused.len(),
        refused.join("\n")
    );
}
