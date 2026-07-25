//! The `Guarded` tier, actually guarding something.
//!
//! These run the whole pipeline, and they have to. What the interpreter checks
//! is exactly what the type checker gave up on, so a test that skipped the
//! checker would be testing a different rule from the one that ships.
//!
//! The bug that made this file: a refined argument was checked and a refined
//! return value was not, so `vow check` printed "so it becomes a runtime check"
//! over a check that did not exist, and the warning was what convinced anyone
//! reading it that they were covered. Every place the checker can record a
//! `Guarded` obligation has a test here, and each one passes a value the guard
//! is supposed to reject. A test that only passes values the guard accepts
//! proves nothing, which is how this went unnoticed.

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::check_text;
use vow_interp::{Program, TestOutcome, codes, run_tests};

/// Checks and runs a source, which must check cleanly.
fn run(src: &str) -> (SourceMap, Vec<TestOutcome>) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.vow", src.to_string());
    assert!(
        !checked.has_errors(),
        "source should check cleanly:\n{}",
        rendered(&sources, &checked.diagnostics)
    );

    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        checked.guards(),
    );
    let outcomes = run_tests(&program, checked.file);
    (sources, outcomes)
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The single failure from a source with exactly one test.
fn expect_refused(src: &str) -> (SourceMap, Diagnostic) {
    let (sources, mut outcomes) = run(src);
    assert_eq!(outcomes.len(), 1, "expected exactly one test");
    let failure = outcomes
        .remove(0)
        .failure
        .expect("the guard should have refused the value");
    assert_eq!(failure.code, codes::REFINEMENT_FAILED);
    (sources, failure)
}

fn expect_pass(src: &str) {
    let (sources, outcomes) = run(src);
    assert!(!outcomes.is_empty(), "no tests were found");
    for outcome in &outcomes {
        if let Some(failure) = &outcome.failure {
            panic!(
                "`{}` should have passed:\n{}",
                outcome.name,
                render_human(&sources, failure)
            );
        }
    }
}

const POSITIVE: &str = "module a\n\ntype Positive = Int where value > 0\n\n";

// -- every way a refined value can come into existence ---------------------

#[test]
fn an_argument_is_guarded() {
    let (sources, failure) = expect_refused(&format!(
        "{POSITIVE}\
         fn take(n: Positive) -> Int {{ n }}\n\n\
         fn indirect(n: Int) -> Int {{ take(n) }}\n\n\
         test \"zero is not positive\" {{\n\
         \x20 assert indirect(0) == 0\n\
         }}\n"
    ));
    let text = render_human(&sources, &failure);
    assert!(text.contains("0 does not satisfy `Positive`"), "{text}");
    assert!(text.contains("could not prove this statically"), "{text}");
}

#[test]
fn a_return_value_is_guarded() {
    // The one that started this. `make(-5)` used to hand back a `-5` that
    // every function downstream was entitled to treat as positive.
    let (sources, failure) = expect_refused(&format!(
        "{POSITIVE}\
         fn make(n: Int) -> Positive {{ n }}\n\n\
         test \"minus five is not positive\" {{\n\
         \x20 assert make(-5) == -5\n\
         }}\n"
    ));
    assert!(render_human(&sources, &failure).contains("-5 does not satisfy `Positive`"));
}

#[test]
fn a_return_from_the_middle_of_a_body_is_guarded() {
    expect_refused(&format!(
        "{POSITIVE}\
         fn make(n: Int) -> Positive {{\n\
         \x20 if n < 100 {{\n\
         \x20   return n\n\
         \x20 }}\n\
         \x20 1\n\
         }}\n\n\
         test \"the early exit counts too\" {{\n\
         \x20 assert make(0) == 0\n\
         }}\n"
    ));
}

#[test]
fn a_branch_of_an_if_is_guarded() {
    // The condition says nothing about being positive either way, so both
    // branches are obligations rather than one being refuted outright.
    expect_refused(&format!(
        "{POSITIVE}\
         fn make(n: Int) -> Positive {{\n\
         \x20 if n > 100 {{\n\
         \x20   n\n\
         \x20 }} else {{\n\
         \x20   n\n\
         \x20 }}\n\
         }}\n\n\
         test \"the else branch counts\" {{\n\
         \x20 assert make(0) == 0\n\
         }}\n"
    ));
}

#[test]
fn a_record_field_is_guarded() {
    expect_refused(&format!(
        "{POSITIVE}\
         record Order {{ quantity: Positive }}\n\n\
         fn order(n: Int) -> Order {{ Order {{ quantity: n }} }}\n\n\
         test \"a field is a boundary too\" {{\n\
         \x20 assert order(0).quantity == 0\n\
         }}\n"
    ));
}

#[test]
fn the_payload_of_ok_is_guarded() {
    expect_refused(&format!(
        "{POSITIVE}\
         fn make(n: Int) -> Result<Positive, String> {{ ok(n) }}\n\n\
         test \"inside a Result still counts\" {{\n\
         \x20 assert make(0) == ok(1)\n\
         }}\n"
    ));
}

#[test]
fn an_annotated_let_is_guarded() {
    expect_refused(&format!(
        "{POSITIVE}\
         fn make(n: Int) -> Int {{\n\
         \x20 let checked: Positive = n\n\
         \x20 checked\n\
         }}\n\n\
         test \"naming it does not launder it\" {{\n\
         \x20 assert make(0) == 0\n\
         }}\n"
    ));
}

#[test]
fn handler_state_is_guarded() {
    // Handler state is the only mutable thing in the language, so it is the
    // one place a refined value can be replaced rather than created.
    expect_refused(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         effect Counter {\n    fn set(to: Int) -> ()\n}\n\n\
         handler InMemory implements Counter {\n\
         \x20 state count: Positive\n\n\
         \x20 fn set(to) -> () {\n\
         \x20   count = to\n\
         \x20 }\n\
         }\n\n\
         test \"state is a boundary too\" {\n\
         \x20 with InMemory { count: 1 } {\n\
         \x20   Counter.set(0)\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn a_handler_operation_parameter_is_guarded() {
    // A handler operation writes no types, because the effect declares them.
    // The effect used never to be consulted, so every parameter in every
    // handler body was the unknown type and nothing done with one was checked.
    expect_refused(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         effect Counter {\n    fn set(to: Positive) -> ()\n}\n\n\
         handler InMemory implements Counter {\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to) -> () {\n\
         \x20   count = to\n\
         \x20 }\n\
         }\n\n\
         fn hide(n: Int) -> Int { n }\n\n\
         test \"the operation's own parameter\" {\n\
         \x20 with InMemory { count: 1 } {\n\
         \x20   Counter.set(hide(0))\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn a_fact_does_not_outlive_the_state_it_was_about() {
    // Handler state is the only thing that can be assigned twice, so it is the
    // only place a fact can survive the value it described. It did: after
    // `count = to` the checker still believed the `count > 0` above, proved
    // `seen` from it, and the guard that would have caught the zero was never
    // emitted.
    expect_refused(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         effect Counter {\n    fn set(to: Int) -> ()\n}\n\n\
         handler InMemory implements Counter {\n\
         \x20 state count: Int\n\n\
         \x20 state seen: Positive\n\n\
         \x20 fn set(to) -> () {\n\
         \x20   if count > 0 {\n\
         \x20     count = to\n\
         \x20     seen = count\n\
         \x20   }\n\
         \x20 }\n\
         }\n\n\
         test \"the fact is stale by the second assignment\" {\n\
         \x20 with InMemory { count: 1, seen: 1 } {\n\
         \x20   Counter.set(0)\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn a_relationship_that_does_not_prove_it_is_guarded() {
    // `low <= high` leaves the difference reaching zero, so the checker records
    // a guard rather than a proof, and the guard has to be real. The pair that
    // does prove it is in proving.rs; this is the one next door to it.
    let (sources, failure) = expect_refused(&format!(
        "{POSITIVE}\
         fn span(low: Int, high: Int) -> Positive\n\
         \x20 where\n\
         \x20   low >= 0,\n\
         \x20   high <= 100,\n\
         \x20   low <= high,\n\
         {{\n\
         \x20 high - low\n\
         }}\n\n\
         test \"an empty span is not positive\" {{\n\
         \x20 assert span(3, 3) == 0\n\
         }}\n"
    ));
    assert!(render_human(&sources, &failure).contains("0 does not satisfy `Positive`"));
}

#[test]
fn a_refinement_over_a_string_is_guarded() {
    // Nothing proves anything about a length, so this check is real. It also
    // needs `length(value)` to be runnable inside a predicate, which it was
    // not: predicates used to be walked by a small interpreter of their own.
    let (sources, failure) = expect_refused(
        "module a\n\n\
         type NonEmpty = String where length(value) > 0\n\n\
         fn shout(s: NonEmpty) -> String { s + \"!\" }\n\n\
         test \"empty is refused\" {\n\
         \x20 assert shout(\"\") == \"!\"\n\
         }\n",
    );
    assert!(render_human(&sources, &failure).contains("NonEmpty"));
}

// -- and the values that should get through --------------------------------

#[test]
fn a_value_that_satisfies_its_refinement_passes_quietly() {
    expect_pass(&format!(
        "{POSITIVE}\
         record Order {{ quantity: Positive }}\n\n\
         fn make(n: Int) -> Positive {{ n }}\n\n\
         fn order(n: Int) -> Order {{ Order {{ quantity: n }} }}\n\n\
         fn wrapped(n: Int) -> Result<Positive, String> {{ ok(n) }}\n\n\
         test \"everything valid\" {{\n\
         \x20 assert make(1) == 1\n\
         \x20 assert order(2).quantity == 2\n\
         \x20 assert wrapped(3) == ok(3)\n\
         }}\n"
    ));
}

#[test]
fn a_string_that_satisfies_its_refinement_passes() {
    expect_pass(
        "module a\n\n\
         type NonEmpty = String where length(value) > 0\n\n\
         fn shout(s: NonEmpty) -> String { s + \"!\" }\n\n\
         test \"a real name\" {\n\
         \x20 assert shout(\"hey\") == \"hey!\"\n\
         }\n",
    );
}

#[test]
fn what_the_checker_proved_is_not_checked_again() {
    // The other half of the tier meaning something. A literal that obviously
    // satisfies the predicate carries no runtime cost, and if it did, the
    // Proven tier would be a comment rather than a claim.
    let mut sources = SourceMap::new();
    let checked = check_text(
        &mut sources,
        "test.vow",
        format!("{POSITIVE}fn make() -> Positive {{ 1 }}\n"),
    );
    assert!(!checked.has_errors());
    assert!(checked.guards().is_empty());
}
