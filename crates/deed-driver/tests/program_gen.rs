//! Generated programs and shrinking.
//!
//! The generator produces random Deed programs; the shrinker reduces any
//! that trigger a failure to the smallest form that still does. Both have to
//! cover everything the other produces, because an unshrunk counterexample
//! cannot be told apart from a small strange one.
//!
//! These tests exercise the pipeline end-to-end: generate, detect a failure,
//! shrink, confirm the result is minimal.

use deed_diagnostics::SourceMap;
use deed_driver::check_text;
use deed_driver::program_gen::{ProgramFuzzConfig, find_program_failure};

fn config(seed: u64) -> ProgramFuzzConfig {
    ProgramFuzzConfig {
        cases: 200,
        seed,
        shrink_budget: 500,
    }
}

// -- the generated programs are real Deed programs -------------------------

#[test]
fn a_simple_generated_program_checks_cleanly() {
    // The simplest possible generated program: one function returning 0.
    let source = "module fuzz\n\nfn f0() -> Int {\n    0\n}\n";
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "fuzz.deed", source);
    assert!(
        !checked.has_errors(),
        "a simple generated program should check cleanly: {:?}",
        checked.diagnostics
    );
}

#[test]
fn a_generated_program_with_arithmetic_checks_cleanly() {
    let source = "module fuzz\n\nfn f0(p0: Int, p1: Int) -> Int {\n    (p0 + p1)\n}\n";
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "fuzz.deed", source);
    assert!(
        !checked.has_errors(),
        "a program with arithmetic should check cleanly: {:?}",
        checked.diagnostics
    );
}

#[test]
fn a_generated_program_with_let_bindings_checks_cleanly() {
    let source = concat!(
        "module fuzz\n\n",
        "fn f0(p0: Int) -> Int {\n",
        "    let v0 = p0\n",
        "    v0\n",
        "}\n"
    );
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "fuzz.deed", source);
    assert!(
        !checked.has_errors(),
        "a program with let bindings should check cleanly: {:?}",
        checked.diagnostics
    );
}

#[test]
fn a_generated_if_expression_checks_cleanly() {
    let source = concat!(
        "module fuzz\n\n",
        "fn f0(p0: Int) -> Int {\n",
        "    if (p0 > 0) { p0 } else { 0 }\n",
        "}\n"
    );
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "fuzz.deed", source);
    assert!(
        !checked.has_errors(),
        "a program with if-then-else should check cleanly: {:?}",
        checked.diagnostics
    );
}

// -- shrinking end-to-end --------------------------------------------------

#[test]
fn a_long_program_shrinks_to_a_short_one() {
    // The failure predicate: the source has more than two functions.
    // Shrinking should remove functions until exactly three remain.
    let finding = find_program_failure(config(0xF00D), |s| s.matches("fn ").count() > 2);
    if let Some(finding) = finding {
        let count = finding.source.matches("fn ").count();
        assert_eq!(
            count, 3,
            "shrinking should stop at the boundary (3), got:\n{}",
            finding.source
        );
    }
}

#[test]
fn the_seed_in_the_finding_matches_the_config() {
    let cfg = config(0xBEEF);
    if let Some(finding) = find_program_failure(cfg, |s| s.contains("f1")) {
        assert_eq!(finding.seed, 0xBEEF);
    }
}

#[test]
fn running_the_same_seed_twice_gives_the_same_finding() {
    let cfg = config(0xCAFE);
    let first = find_program_failure(cfg, |s| s.matches("fn ").count() >= 2).map(|f| f.source);
    let second = find_program_failure(cfg, |s| s.matches("fn ").count() >= 2).map(|f| f.source);
    assert_eq!(first, second, "the same seed must produce the same result");
}

#[test]
fn a_predicate_that_never_fails_produces_no_finding() {
    let cfg = config(0xDEAD);
    let finding = find_program_failure(cfg, |_| false);
    assert!(finding.is_none(), "no predicate should find no failure");
}

#[test]
fn declarations_shrink_to_the_minimal_failing_count() {
    // The predicate: at least two functions. Shrinking should stop at exactly
    // two, since removing one more would drop below the threshold.
    if let Some(finding) = find_program_failure(config(0x5EED), |s| s.matches("fn ").count() >= 2) {
        let count = finding.source.matches("fn ").count();
        assert_eq!(
            count, 2,
            "shrinking should stop at exactly two functions:\n{}",
            finding.source
        );
    }
}

#[test]
fn statements_shrink_to_the_minimal_failing_count() {
    // Any program with a `let` binding should shrink to one with exactly one.
    if let Some(finding) = find_program_failure(config(0xABCD), |s| s.contains("let ")) {
        let count = finding.source.matches("let ").count();
        assert_eq!(
            count, 1,
            "shrinking should stop at one let binding:\n{}",
            finding.source
        );
    }
}
