//! `for`, which is a fold with syntax rather than a loop with a variable in it.
//!
//! The decision this file is the consequence of is #90. Vow has one mutable
//! thing, a handler's `state`, and an accumulator loop wants either mutation
//! or a fold. Choosing the fold is what lets iteration exist without the
//! language having a second mutable thing in it, and choosing anything at all
//! is what stops every walk over a list declaring `Diverge`.

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::check_text;
use vow_interp::{Program, TestOutcome, run_tests};

fn check(src: &str) -> (SourceMap, vow_driver::Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.vow", src);
    (sources, checked)
}

fn check_ok(src: &str) {
    let (sources, checked) = check(src);
    if !checked.diagnostics.is_empty() {
        panic!(
            "expected a clean check:\n{}",
            rendered(&sources, &checked.diagnostics)
        );
    }
}

fn codes_of(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code).collect()
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

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
        checked.rows(),
    );
    let outcomes = run_tests(&program, checked.file);
    (sources, outcomes)
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

// -- the point of it --------------------------------------------------------

#[test]
fn walking_a_list_does_not_declare_diverge() {
    // This is the whole reason iteration was worth arguing about. Recursion is
    // the alternative, a function that can reach itself has to declare that it
    // may not return, and walking a list is the most ordinary thing a program
    // does. A row that every function has to carry the same entry in is a row
    // that has stopped saying anything.
    check_ok(
        "module a\n\n\
         fn total(numbers: List<Int>) -> Int {\n\
         \x20 for n in numbers with sum = 0 {\n\
         \x20   sum + n\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn declaring_diverge_for_a_for_is_refused() {
    // The other half of the same rule. Declaring an effect the body cannot
    // perform is an error, so a `for` cannot quietly keep the habit alive.
    let (_, checked) = check(
        "module a\n\n\
         fn total(numbers: List<Int>) -> Int\n\
         \x20 uses\n\
         \x20   Diverge,\n\
         {\n\
         \x20 for n in numbers with sum = 0 {\n\
         \x20   sum + n\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_effects::codes::UNUSED_EFFECT]
    );
}

// -- types ------------------------------------------------------------------

#[test]
fn the_binder_has_the_element_type() {
    check_ok(
        "module a\n\n\
         fn f(words: List<String>) -> Int {\n\
         \x20 for word in words with total = 0 {\n\
         \x20   total + length(word)\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn the_body_has_to_produce_the_accumulator_again() {
    // Its value is what the next turn starts with, so anything else would be a
    // fold whose two halves disagree about what is being folded.
    let (_, checked) = check(
        "module a\n\n\
         fn f(numbers: List<Int>) -> Int {\n\
         \x20 for n in numbers with sum = 0 {\n\
         \x20   \"not a number\"\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn without_an_accumulator_the_body_produces_unit() {
    check_ok(
        "module a\n\n\
         effect Log {\n    fn note(text: String) -> ()\n}\n\n\
         fn f(words: List<String>) -> ()\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {\n\
         \x20 for word in words {\n\
         \x20   Log.note(word)\n\
         \x20 }\n\
         }\n",
    );

    let (_, checked) = check(
        "module a\n\n\
         fn f(numbers: List<Int>) -> () {\n\
         \x20 for n in numbers {\n\
         \x20   n\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn there_is_one_thing_to_walk() {
    let (sources, checked) = check(
        "module a\n\n\
         fn f(text: String) -> Int {\n\
         \x20 for c in text with n = 0 {\n\
         \x20   n + 1\n\
         \x20 }\n\
         }\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::NOT_A_LIST),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn the_accumulator_is_not_in_scope_in_what_it_starts_as() {
    // `with sum = sum` names something that does not exist yet, rather than a
    // value that refers to itself.
    let (_, checked) = check(
        "module a\n\n\
         fn f(numbers: List<Int>) -> Int {\n\
         \x20 for n in numbers with sum = sum {\n\
         \x20   sum + n\n\
         \x20 }\n\
         }\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_resolve::codes::UNKNOWN_NAME),
        "{:?}",
        codes_of(&checked.diagnostics)
    );
}

#[test]
fn the_binder_is_a_binding_like_any_other() {
    // Shadowing is an error everywhere else, and a `for` does not get an
    // exemption from the rule that a name means one thing.
    let (_, checked) = check(
        "module a\n\n\
         fn f(n: Int, numbers: List<Int>) -> Int {\n\
         \x20 for n in numbers with sum = 0 {\n\
         \x20   sum + n\n\
         \x20 }\n\
         }\n",
    );
    assert!(!checked.diagnostics.is_empty(), "shadowing went unnoticed");
}

// -- running ----------------------------------------------------------------

#[test]
fn a_fold_over_numbers() {
    expect_pass(
        "module a\n\n\
         fn total(numbers: List<Int>) -> Int {\n\
         \x20 for n in numbers with sum = 0 {\n\
         \x20   sum + n\n\
         \x20 }\n\
         }\n\n\
         test \"adding up\" {\n\
         \x20 assert total([1, 2, 3]) == 6\n\
         \x20 assert total([7]) == 7\n\
         }\n",
    );
}

#[test]
fn an_empty_list_produces_what_the_accumulator_started_as() {
    // The body never runs, so the answer is the initial value and not a
    // special case anybody has to write.
    expect_pass(
        "module a\n\n\
         fn total(numbers: List<Int>) -> Int {\n\
         \x20 for n in numbers with sum = 41 {\n\
         \x20   sum + n\n\
         \x20 }\n\
         }\n\n\
         test \"nothing to fold\" {\n\
         \x20 assert total([]) == 41\n\
         }\n",
    );
}

#[test]
fn a_fold_can_build_a_list() {
    expect_pass(
        "module a\n\n\
         fn shout(words: List<String>) -> List<String> {\n\
         \x20 for word in words with loud = [] {\n\
         \x20   push(loud, word + \"!\")\n\
         \x20 }\n\
         }\n\n\
         test \"building\" {\n\
         \x20 assert shout([\"a\", \"b\"]) == [\"a!\", \"b!\"]\n\
         \x20 assert shout([]) == []\n\
         }\n",
    );
}

#[test]
fn the_accumulator_is_bound_again_rather_than_assigned() {
    // The distinction the whole design rests on. Each turn sees the value the
    // previous turn produced, and nothing anywhere was mutated to do it.
    expect_pass(
        "module a\n\n\
         fn joined(words: List<String>) -> String {\n\
         \x20 for word in words with text = \"\" {\n\
         \x20   text + word\n\
         \x20 }\n\
         }\n\n\
         test \"carried\" {\n\
         \x20 assert joined([\"a\", \"b\", \"c\"]) == \"abc\"\n\
         }\n",
    );
}

#[test]
fn a_for_can_be_nested_in_a_for() {
    expect_pass(
        "module a\n\n\
         fn total(rows: List<List<Int>>) -> Int {\n\
         \x20 for row in rows with sum = 0 {\n\
         \x20   for n in row with inner = sum {\n\
         \x20     inner + n\n\
         \x20   }\n\
         \x20 }\n\
         }\n\n\
         test \"two deep\" {\n\
         \x20 assert total([[1, 2], [3]]) == 6\n\
         \x20 assert total([]) == 0\n\
         }\n",
    );
}

#[test]
fn a_for_with_no_accumulator_runs_for_its_effects() {
    expect_pass(
        "module a\n\n\
         effect Log {\n    fn note(text: String) -> ()\n}\n\n\
         handler Collect implements Log {\n\
         \x20 state seen: String\n\n\
         \x20 fn note(text) -> () {\n\
         \x20   seen = seen + text\n\
         \x20 }\n\
         }\n\n\
         fn shout(words: List<String>) -> ()\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {\n\
         \x20 for word in words {\n\
         \x20   Log.note(word)\n\
         \x20 }\n\
         }\n\n\
         test \"for its effects\" {\n\
         \x20 with Collect { seen: \"\" } {\n\
         \x20   shout([\"a\", \"b\"])\n\
         \x20 }\n\
         }\n",
    );
}
