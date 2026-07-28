//! Every warning the corpus produces is one it means to produce.
//!
//! `examples/` is what somebody reads first and what `deed check examples`
//! reports on, so a warning in it is a warning on the shipped work. Some are
//! the point: `proven.deed` and `strings.deed` exist partly to show an
//! obligation landing in the `Guarded` tier, and that warning is the
//! demonstration rather than a blemish on it.
//!
//! The rest are not. `strings.deed` named a parameter `line` next door to a
//! function called `line`, which shadowed it, which meant `words` could not
//! have called `line` even if it wanted to. Nothing said that was deliberate
//! and nothing noticed it either, because every other test over this corpus
//! asserts `!has_errors()` and a warning is not an error.
//!
//! So the rule is written down here rather than left to whoever next runs the
//! command by hand. A new kind of warning in the corpus fails, and the way to
//! pass is either to fix the program or to argue here that the corpus means
//! to say it.

use std::fs;
use std::path::PathBuf;

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::{check_all, shipped_modules, shipped_source};

/// Every `.deed` file in `examples/`, checked together.
///
/// Together because they import each other, and a name with nothing behind it
/// is a different diagnostic than the one this is about.
fn corpus() -> (SourceMap, Vec<Diagnostic>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples");

    let mut paths: Vec<PathBuf> = fs::read_dir(&root)
        .expect("examples/ should be there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "deed"))
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "no `.deed` files under {}",
        root.display()
    );

    let mut sources = SourceMap::new();
    let mut ids: Vec<_> = paths
        .iter()
        .map(|path| {
            let text = fs::read_to_string(path).expect("an example should be readable");
            let name = format!(
                "examples/{}",
                path.file_name().expect("a file").to_string_lossy()
            );
            sources.add(name, text)
        })
        .collect();

    // The corpus imports a module that ships inside the compiler, so it is
    // here for the same reason the command line puts it there. Last, and the
    // diagnostics below are read off the first `subject` of them: what this
    // test is about is the warnings `examples/` produces, and a library
    // shipping with the compiler is checked by `shipped.rs`.
    let subject = ids.len();
    for module in shipped_modules() {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }

    let said = check_all(&sources, &ids)[..subject]
        .iter()
        .flat_map(|one| one.diagnostics.clone())
        .collect();
    (sources, said)
}

#[test]
fn the_only_warning_the_corpus_means_to_produce_is_a_guarded_obligation() {
    let (sources, said) = corpus();

    // Errors are somebody else's test. This one is about what is left when
    // there are none, which is the part nothing was looking at.
    let unexpected: Vec<&Diagnostic> = said
        .iter()
        .filter(|d| !d.is_error() && d.code != deed_typeck::codes::UNPROVEN_REFINEMENT)
        .collect();

    assert!(
        unexpected.is_empty(),
        "the corpus warns about something it does not mean to:\n{}",
        unexpected
            .iter()
            .map(|d| render_human(&sources, d))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// And the demonstration is still there.
///
/// Without this the test above passes on a corpus that stopped warning at all,
/// which would mean the tiers had quietly stopped being visible in the one
/// place they are shown.
#[test]
fn the_guarded_tier_is_still_demonstrated() {
    let (_, said) = corpus();
    let guarded = said
        .iter()
        .filter(|d| d.code == deed_typeck::codes::UNPROVEN_REFINEMENT)
        .count();
    assert!(
        guarded > 0,
        "no obligation in the corpus lands in `Guarded`, so nothing shows what that tier is"
    );
}
