//! The program held to its own signatures while it runs.
//!
//! A row says what a function may perform. Until this existed, the only thing
//! that ever read one was the pass that wrote it, so a hole in that pass was a
//! hole nothing could see. Five separate ways of getting an effect past it
//! were open at once, and each was found by hand.
//!
//! So the run checks too. Every effect performed is held against every call on
//! the stack that did not already discharge it with a `with` block, and a
//! function that performs something it did not declare is `VOW6010`. That is
//! reported against the compiler rather than the program: the file was
//! accepted, so if an effect got through then the check was wrong.
//!
//! This is the same move as `crates/vow-driver/tests/fully_typed.rs`, one pass
//! along. That one says no expression in a clean file is untyped. This one says
//! nothing a clean file does is undeclared.

use std::fs;
use std::path::{Path, PathBuf};

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::check_all;
use vow_interp::{Program, run_tests};

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Checks every file together and runs the tests in all of them.
///
/// Together because a row crosses a module boundary, and the interesting case
/// is a caller in one file and a callback in another.
fn run_all(files: &[(String, String)]) -> (SourceMap, Vec<Diagnostic>) {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = files
        .iter()
        .map(|(name, text)| sources.add(name.clone(), text.clone()))
        .collect();
    let checked = check_all(&sources, &ids);

    let failures: Vec<Diagnostic> = checked
        .iter()
        .flat_map(|one| one.diagnostics.iter().filter(|d| d.is_error()).cloned())
        .collect();
    assert!(
        failures.is_empty(),
        "these should have checked cleanly:\n{}",
        rendered(&sources, &failures)
    );

    let mut program = Program::new();
    for one in &checked {
        program.add(
            one.file,
            &one.module,
            &one.resolutions,
            one.guards(),
            one.rows(),
        );
    }

    let mut said = Vec::new();
    for one in &checked {
        for outcome in run_tests(&program, one.file) {
            said.extend(outcome.failure);
        }
    }
    (sources, said)
}

/// Every `.vow` file in `examples/`, which is where the real programs are.
fn examples() -> Vec<(String, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples");
    let mut found: Vec<PathBuf> = fs::read_dir(&root)
        .expect("examples/ should be there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "vow"))
        .collect();
    found.sort();

    found
        .iter()
        .map(|path| {
            let name = Path::new("examples")
                .join(path.file_name().expect("a file"))
                .to_string_lossy()
                .replace('\\', "/");
            (name, fs::read_to_string(path).expect("readable"))
        })
        .collect()
}

#[test]
fn every_example_keeps_the_rows_it_declared() {
    // The corpus is the point. These are the programs that exercise closures,
    // callbacks, handlers, capabilities and a library written in Vow, so they
    // are where a hole in the effect checker would actually show up.
    let (sources, said) = run_all(&examples());
    let broken: Vec<Diagnostic> = said
        .into_iter()
        .filter(|d| d.code == vow_interp::codes::ROW_NOT_KEPT)
        .collect();
    assert!(
        broken.is_empty(),
        "an effect got past the checker:\n{}",
        rendered(&sources, &broken)
    );
}

/// A module with a `Log` effect, a handler that counts, and two functions.
const LOG: &str = "module log\n\n\
     effect Log {\n\
     \x20 fn note(message: String) -> Int\n\
     }\n\n\
     handler Counted implements Log {\n\
     \x20 state seen: Int\n\n\
     \x20 fn note(message) -> Int {\n\
     \x20   seen = seen + 1\n\
     \x20   seen\n\
     \x20 }\n\
     }\n\n\
     fn logs(n: Int) -> Int uses Log.note { n + Log.note(\"x\") }\n\n";

#[test]
fn a_declared_effect_is_not_reported() {
    // The check has to be quiet about the ordinary case, or the corpus test
    // above would only be saying that nothing runs.
    let (sources, said) = run_all(&[(
        "log.vow".to_string(),
        format!(
            "{LOG}\
             test \"it runs\" {{\n\
             \x20 with Counted {{ seen: 0 }} {{\n\
             \x20   assert logs(1) == 2\n\
             \x20 }}\n\
             }}\n"
        ),
    )]);
    assert!(said.is_empty(), "{}", rendered(&sources, &said));
}

#[test]
fn a_with_block_inside_the_function_discharges_it() {
    // The frame holding the `with` does not owe the effect, and the frames
    // inside it do. Getting this backwards would make every handled effect a
    // false report, which is the failure mode that would make the whole check
    // useless.
    let (sources, said) = run_all(&[(
        "log.vow".to_string(),
        format!(
            "{LOG}\
             fn quietly(n: Int) -> Int {{\n\
             \x20 with Counted {{ seen: 0 }} {{\n\
             \x20   logs(n)\n\
             \x20 }}\n\
             }}\n\n\
             test \"it runs\" {{\n\
             \x20 assert quietly(1) == 2\n\
             }}\n"
        ),
    )]);
    assert!(said.is_empty(), "{}", rendered(&sources, &said));
}

#[test]
fn a_contract_may_read_state_without_declaring_it() {
    // A `where` or `ensures` clause describes state rather than changing it,
    // and a contract does not contribute to a row. So this is the one place an
    // effect happens and no signature has to admit to it, and the check has to
    // know that or `examples/transfer.vow` fails on its own postcondition.
    let (sources, said) = run_all(&[(
        "log.vow".to_string(),
        format!(
            "{LOG}\
             fn counted(n: Int) -> Int\n\
             \x20 uses Log.note,\n\
             \x20 ensures\n\
             \x20   ok  => Log.note(\"asking\") > 0,\n\
             \x20   err => true,\n\
             {{ n + Log.note(\"working\") }}\n\n\
             test \"it runs\" {{\n\
             \x20 with Counted {{ seen: 0 }} {{\n\
             \x20   assert counted(1) == 2\n\
             \x20 }}\n\
             }}\n"
        ),
    )]);
    assert!(said.is_empty(), "{}", rendered(&sources, &said));
}
