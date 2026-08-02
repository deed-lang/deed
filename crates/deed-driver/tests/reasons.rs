//! What the corpus's obligations actually are, counted by tier and by reason.
//!
//! Part of #602. Once `Unknown` carried a [`Reason`], the natural next
//! question is which reasons the corpus actually produces, because that
//! count is what tells the next move apart from a guess: mostly "nothing
//! narrowed this name" points at documentation and messages, mostly "not the
//! shape the checker reasons about" makes the solver question (#601) an
//! argument the evidence can settle, and mostly "crossed a module boundary"
//! points at the surface carrying more, which is scoped work already.
//!
//! This walks `examples/` and the shipped `std/` modules together, the same
//! corpus `corpus.rs` and `shipped.rs` already check, and tallies every
//! obligation `deed check --obligations` would report about them.

use std::fs;
use std::path::PathBuf;

use deed_driver::{ObligationReport, check_all, shipped_modules, shipped_source};
use deed_typeck::{Reason, Tier};

fn corpus_and_library() -> Vec<ObligationReport> {
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

    let mut sources = deed_diagnostics::SourceMap::new();
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
    for module in shipped_modules() {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }

    let checks = check_all(&sources, &ids);
    for checked in &checks {
        assert!(
            !checked.has_errors(),
            "{} should check cleanly",
            sources.file(checked.file).name()
        );
    }

    checks
        .into_iter()
        .flat_map(|checked| checked.obligations)
        .collect()
}

fn reason_text(reason: Reason) -> &'static str {
    reason.text()
}

/// One row of the table: how many obligations, of each tier, and of the
/// `Guarded` ones, how many carry each reason.
///
/// A ratchet rather than a `>=`, because a number that only ever grows is a
/// number nobody has to explain when it changes, and this one is the evidence
/// #601 argues from. If this fails after adding or changing a program in the
/// corpus, update the counts here and in `design/02-syntax.md` together, and
/// say in the same commit whether the new counts still point the same way.
#[test]
fn the_corpus_is_counted_by_tier_and_by_reason() {
    let obligations = corpus_and_library();

    let proven = obligations
        .iter()
        .filter(|o| o.tier == Tier::Proven)
        .count();
    let tested = obligations
        .iter()
        .filter(|o| o.tier == Tier::Tested)
        .count();
    let guarded: Vec<&ObligationReport> = obligations
        .iter()
        .filter(|o| o.tier == Tier::Guarded)
        .collect();

    let name_not_narrowed = guarded
        .iter()
        .filter(|o| o.reason.map(reason_text) == Some(Reason::NothingNarrowedThisName.text()))
        .count();
    let length_not_established = guarded
        .iter()
        .filter(|o| o.reason.map(reason_text) == Some(Reason::NothingEstablishedThisLength.text()))
        .count();
    let value_unnamed = guarded
        .iter()
        .filter(|o| o.reason.map(reason_text) == Some(Reason::NothingNamesThisValue.text()))
        .count();
    let crossed_a_boundary = guarded
        .iter()
        .filter(|o| o.reason.map(reason_text) == Some(Reason::CrossedAModuleBoundary.text()))
        .count();
    let not_a_shape = guarded
        .iter()
        .filter(|o| {
            o.reason.map(reason_text) == Some(Reason::NotAShapeTheCheckerReasonsAbout.text())
        })
        .count();
    let nothing_tries = guarded
        .iter()
        .filter(|o| o.reason.map(reason_text) == Some(Reason::NothingTriesToProveThis.text()))
        .count();
    let no_reason_at_all = guarded.iter().filter(|o| o.reason.is_none()).count();

    // Every Guarded obligation is accounted for by exactly one of the buckets
    // above, so the parts add up to the whole without a bucket for "counted
    // twice" or "counted nowhere" to hide in.
    //
    // `no_reason_at_all` is now zero and `crates/deed-driver/tests/obligations.rs`
    // holds it there. It used to be nine, all of them `ensures` clauses, which
    // `check_all` never routes through `facts::holds`: nothing tries to settle
    // one ahead of time, so the floor is Guarded whatever the body looks like.
    // Saying nothing made that look like the same answer as "I looked and could
    // not", so those nine now carry `NothingTriesToProveThis` instead.
    assert_eq!(
        name_not_narrowed
            + length_not_established
            + value_unnamed
            + crossed_a_boundary
            + not_a_shape
            + nothing_tries
            + no_reason_at_all,
        guarded.len(),
        "every Guarded obligation should fall into exactly one reason bucket, including \"none\""
    );

    assert_eq!(
        (
            proven,
            tested,
            name_not_narrowed,
            length_not_established,
            value_unnamed,
            crossed_a_boundary,
            not_a_shape,
            nothing_tries,
            no_reason_at_all,
        ),
        (72, 8, 2, 1, 0, 0, 0, 9, 0),
        "the corpus's obligation counts changed; update this test and the table in \
         design/02-syntax.md together"
    );
}
