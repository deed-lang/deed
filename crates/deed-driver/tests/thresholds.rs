//! The thresholds that would reopen a closed decision.
//!
//! `design/refusals.md` closes eleven questions, and six of them end with a
//! paragraph starting "What would change the answer". Those paragraphs are the
//! most valuable thing on that page and the least protected: the refusals
//! themselves are read by tests, so a `trait` keyword appearing would be
//! noticed, but nothing reads the thresholds. If the thing that would reopen a
//! decision quietly became true, the page would go on saying the decision was
//! settled and the reasoning would be a museum piece.
//!
//! These hold the thresholds that can be checked by running something. A
//! threshold like "an explicit session model" is a design somebody has to
//! write rather than a condition that becomes true on its own, and those are
//! left alone rather than given a test that pretends to watch them.
//!
//! Every test here fails *when the decision should be reopened*, which is the
//! opposite direction from most of this suite. A failure is not a regression;
//! it is the page asking to be rewritten.

use deed_diagnostics::SourceMap;
use deed_driver::{Checked, check_all, shipped_for, shipped_source};
use deed_interp::{Program, run_tests};

/// Checks one program together with whatever it imports from the shipped
/// library, and runs its tests.
///
/// Returns the names of the tests that failed, so a caller can say what it
/// expected rather than only that something went wrong.
fn outcome(source: &str) -> Result<Vec<String>, String> {
    let mut sources = SourceMap::new();
    let subject = sources.add("threshold.deed".to_string(), source.to_string());
    let mut ids = vec![subject];

    for module in shipped_for([source]) {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("{module}.deed"), text.to_string()));
    }

    let checks: Vec<Checked> = check_all(&sources, &ids);
    let complaints: Vec<String> = checks
        .iter()
        .flat_map(|one| one.diagnostics.iter())
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect();
    if !complaints.is_empty() {
        return Err(complaints.join("\n"));
    }

    let mut program = Program::new();
    for one in &checks {
        program.add(
            one.file,
            &one.module,
            &one.resolutions,
            one.guards(),
            one.rows(),
        );
    }

    let outcomes = run_tests(&program, subject);
    assert!(!outcomes.is_empty(), "the probe declared no tests");

    Ok(outcomes
        .into_iter()
        .filter(|outcome| outcome.failure.is_some())
        .map(|outcome| outcome.name)
        .collect())
}

/// No traits: "a program that needs a generic sort over a user type, or needs
/// to print a `T`, where a passed function is not merely uglier but
/// impossible."
///
/// Both halves, over types that ship rather than over types invented for the
/// test. `Ratio` has no ordering the language knows about and `Date` has no
/// rendering, so if a passed function were not enough, this is where it would
/// show.
#[test]
fn a_passed_function_still_does_what_a_trait_would_have() {
    let failed = outcome(
        "module threshold/traits\n\n\
         use std/date.{Date, text}\n\
         use std/list.{map, sort}\n\
         use std/ratio.{Ratio, is_below, simplified}\n\n\
         fn ranked(values: List<Ratio>) -> List<Ratio> {\n\
         \x20   sort(values, |a: Ratio, b: Ratio| is_below(a, b))\n\
         }\n\n\
         fn shown<T, uses r>(values: List<T>, render: Fn(T) uses r -> String) -> String\n\
         \x20 uses\n\
         \x20   r,\n\
         {\n\
         \x20   join(map(values, render), \", \")\n\
         }\n\n\
         test \"a generic sort over a user type\" {\n\
         \x20   let sorted = ranked([simplified(2, 3), simplified(1, 6), simplified(1, 2)])\n\
         \x20   assert sorted == [simplified(1, 6), simplified(1, 2), simplified(2, 3)]\n\
         }\n\n\
         test \"printing a T\" {\n\
         \x20   let dates = [Date { year: 2024, month: 3, day: 1 }]\n\
         \x20   assert shown(dates, text) == \"2024-03-01\"\n\
         }\n",
    );

    match failed {
        Ok(failed) => assert!(
            failed.is_empty(),
            "a passed function no longer does the job: {failed:?}. \
             `design/refusals.md`'s trait threshold has been reached."
        ),
        Err(complaints) => panic!(
            "a generic sort or a passed renderer over a shipped type no longer checks:\n\
             {complaints}\n\
             That is `design/refusals.md`'s trait threshold, and the decision should be reopened."
        ),
    }
}

/// No fractional numbers: the remaining cost is that `1/2 + 1/3` has to be
/// spelled `added(half, third)`, which is an argument about operator
/// overloading rather than about numbers.
///
/// The other half of that page's threshold is a contract that has to say
/// something about a fractional quantity. Nothing in `std/ratio` does, which is
/// why the proof model was never asked anything, and this counts it rather than
/// asserting it.
#[test]
fn nothing_yet_writes_a_contract_about_a_fractional_quantity() {
    let text = shipped_source("std/ratio").expect("std/ratio ships");
    let clauses = text
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            line.starts_with("ensures") || line.starts_with("where")
        })
        .count();

    assert_eq!(
        clauses, 0,
        "`std/ratio` has grown {clauses} contract clauses, so the half of \
         `design/fractional-values.md`'s threshold about contracts over fractional \
         quantities has been reached and the decision should be reopened"
    );
}

/// `Result` and `List` stay in the language: "a rule saying which variant of a
/// two-variant choice means stop", and "something for `for` to walk that is not
/// spelled `List`".
///
/// The second half is the one a test can watch. `for` walking anything else
/// would be the second walkable thing that decision is about.
#[test]
fn for_still_walks_a_list_and_nothing_else() {
    let refused = outcome(
        "module threshold/walk\n\n\
         fn over_text(line: String) -> Int {\n\
         \x20   for character in line with seen = 0 {\n\
         \x20       seen + 1\n\
         \x20   }\n\
         }\n\n\
         test \"unreachable\" {\n\
         \x20   assert over_text(\"ab\") == 2\n\
         }\n",
    );

    assert!(
        refused.is_err(),
        "`for` now walks a `String`, so there is a second walkable thing and \
         `design/refusals.md`'s `Result`/`List` threshold has been reached"
    );
}

/// No detached spawn: `spawn(f())` at statement level is refused.
///
/// The threshold is a program scoped concurrency cannot express, which is not
/// a condition a test can watch. What it can watch is the refusal still being
/// a refusal, because a `spawn` that started parsing as something else would
/// make the page's claim about `DEED2014` untrue without anybody noticing.
#[test]
fn a_detached_spawn_is_still_refused_by_name() {
    let refused = outcome(
        "module threshold/spawn\n\n\
         fn work() -> Int {\n\
         \x20   1\n\
         }\n\n\
         fn start() -> () {\n\
         \x20   spawn(work())\n\
         }\n\n\
         test \"unreachable\" {\n\
         \x20   assert start() == ()\n\
         }\n",
    );

    let Err(complaints) = refused else {
        panic!("a detached spawn was accepted, so the structured-concurrency decision is stale");
    };
    assert!(
        complaints.contains("DEED2014"),
        "a detached spawn is refused for some other reason now: {complaints}"
    );
}

/// Every threshold that a test could watch has one.
///
/// The page states six. Three are conditions somebody would have to build
/// rather than conditions that become true (a session model for the REPL, a
/// realistic codebase for incremental checking, a program scoped concurrency
/// cannot express), and inventing a test for those would be a test that watches
/// nothing. The other three are here.
///
/// This counts, so that a seventh threshold arriving is a decision about
/// whether it can be watched rather than something nobody looked at.
#[test]
fn the_page_states_the_number_of_thresholds_this_file_was_written_against() {
    let page = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/refusals.md"),
    )
    .expect("the refusals page should be readable");

    let stated = page.matches("**What would change the answer").count();
    assert_eq!(
        stated, 6,
        "`design/refusals.md` states {stated} thresholds rather than the six \
         `crates/deed-driver/tests/thresholds.rs` was written against; a new one \
         needs either a test here or a sentence saying why it cannot have one"
    );
}
