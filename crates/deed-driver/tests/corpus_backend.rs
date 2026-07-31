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
use deed_driver::{check_all, shipped_modules, shipped_source};

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
        let mut ids = vec![map.add(name.clone(), text)];

        // A file that imports something needs it alongside, the same way the
        // command line hands the compiler both.
        for module in shipped_modules() {
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

        let refused = match deed_mir::lower(&subject.module, &subject.resolutions, &subject.types) {
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

/// The corpus is mostly beyond this backend, and that is written down rather
/// than discovered.
///
/// A floor rather than a target, and named rather than counted: a list says
/// which file stopped compiling, and a number only says that one did. What
/// it catches is a change that quietly stops compiling things that used to,
/// which is the direction a backend regresses in.
///
/// #623: the other direction regresses just as quietly, so this checks it
/// too. Every file the backend compiles today has to be in the list below,
/// not only every file in the list has to compile: a file that starts
/// compiling and is not added here passes silently, and six months later
/// nobody remembers whether that was on purpose. Growing this list is the
/// last commit of whatever PR made the file compile, not a cleanup
/// afterwards.
///
/// The files not in this list are refused for one of two reasons, both
/// tracked as their own issue rather than repeated here per file: a prelude
/// function the backend does not compile yet (#621, everything but `ok`,
/// `err` and `length`), or early `return` (#620, `counter.deed`,
/// `transfer.deed` and `proven.deed` specifically). A file refused for a
/// third reason should say so in `every_refusal_says_what_it_found`'s
/// output before it is worth chasing further.
#[test]
fn the_backend_still_compiles_what_it_used_to() {
    let outcomes = walked();
    let mut compiled: Vec<&str> = outcomes
        .iter()
        .filter(|one| one.refused.is_none())
        .map(|one| one.name.as_str())
        .collect();
    compiled.sort_unstable();

    let mut expected = vec![
        "calendar.deed",
        // The capability example: a program that reads a file it was given
        // access to and cannot read one it was not.
        "config.deed",
        "diverge.deed",
        "generics.deed",
        // A program that writes a line, which the module asks its host for.
        "hello.deed",
        "list.deed",
        "names.deed",
        "table.deed",
    ];
    expected.sort_unstable();

    for name in &expected {
        assert!(
            compiled.contains(name),
            "`{name}` used to compile and does not any more; the corpus now compiles {compiled:?}"
        );
    }

    assert_eq!(
        compiled, expected,
        "the corpus now compiles a file this list does not name; add it to `expected` \
         in the same change that made it compile, so the next reader does not have to \
         rediscover which files the backend handles"
    );
}
