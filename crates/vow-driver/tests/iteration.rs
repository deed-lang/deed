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

// -- where in the list it is ------------------------------------------------
//
// A `for` used to walk a list without saying where in it you were, so the
// moment a position mattered the walk went back to a counter carried in a
// record, and every branch had to remember to bump it. There were three of
// those in `examples/todo.vow` and they were the same three lines each time.

#[test]
fn the_index_is_an_integer_and_counts_from_zero() {
    expect_pass(
        "module a\n\n\
         fn positions(items: List<String>) -> List<Int> {\n\
         \x20 for item at here in items with out = [] {\n\
         \x20   push(out, here)\n\
         \x20 }\n\
         }\n\n\
         test \"from zero\" {\n\
         \x20 assert positions([\"a\", \"b\", \"c\"]) == [0, 1, 2]\n\
         \x20 assert positions([]) == []\n\
         }\n",
    );
}

#[test]
fn the_index_is_bound_again_on_every_turn_like_everything_else() {
    expect_pass(
        "module a\n\n\
         fn weighted(numbers: List<Int>) -> Int {\n\
         \x20 for n at here in numbers with total = 0 {\n\
         \x20   total + n * here\n\
         \x20 }\n\
         }\n\n\
         test \"nothing is assigned\" {\n\
         \x20 assert weighted([5, 5, 5]) == 15\n\
         }\n",
    );
}

#[test]
fn the_index_is_a_binding_like_any_other() {
    // Shadowing is an error everywhere else, and this is not an exemption.
    let (_, checked) = check(
        "module a\n\n\
         fn f(here: Int, numbers: List<Int>) -> Int {\n\
         \x20 for n at here in numbers with sum = 0 {\n\
         \x20   sum + n\n\
         \x20 }\n\
         }\n",
    );
    assert!(!checked.diagnostics.is_empty(), "shadowing went unnoticed");
}

#[test]
fn the_index_is_known_to_be_a_real_position() {
    // Not negative and below the length of what is being walked, which is what
    // makes it worth binding rather than counting by hand: something that
    // indexes with it can say so and be believed.
    check_ok(
        "module a\n\n\
         fn nth(items: List<Int>, index: Int) -> Result<Int, String>\n\
         \x20 where\n\
         \x20   index >= 0,\n\
         \x20   index < length(items),\n\
         {\n\
         \x20 at(items, index)\n\
         }\n\n\
         fn each(items: List<Int>) -> List<Result<Int, String>> {\n\
         \x20 for item at here in items with out = [] {\n\
         \x20   push(out, nth(items, here))\n\
         \x20 }\n\
         }\n",
    );
    let (_, checked) = check(
        "module a\n\n\
         fn nth(items: List<Int>, index: Int) -> Result<Int, String>\n\
         \x20 where\n\
         \x20   index >= 0,\n\
         \x20   index < length(items),\n\
         {\n\
         \x20 at(items, index)\n\
         }\n\n\
         fn each(items: List<Int>) -> List<Result<Int, String>> {\n\
         \x20 for item at here in items with out = [] {\n\
         \x20   push(out, nth(items, here))\n\
         \x20 }\n\
         }\n",
    );
    let proven = checked
        .obligations
        .iter()
        .filter(|obligation| obligation.tier == vow_typeck::Tier::Proven)
        .count();
    assert_eq!(proven, 2, "obligations: {:?}", checked.obligations);
}

#[test]
fn a_for_that_does_not_ask_where_it_is_binds_nothing_extra() {
    // `at` is a name everywhere else, including the prelude function that
    // indexes a list, so leaving it off has to leave the walk alone.
    expect_pass(
        "module a\n\n\
         fn heads(items: List<Int>) -> Result<Int, String> {\n\
         \x20 for item in items with found = at(items, 0) {\n\
         \x20   found\n\
         \x20 }\n\
         }\n\n\
         test \"at is still a function\" {\n\
         \x20 assert heads([4, 5]) == ok(4)\n\
         }\n",
    );
}

// -- stopping early ---------------------------------------------------------
//
// `while` is read before each turn with the accumulator in scope. It is not
// `break`: nothing is abandoned, the walk just does not take the turn, and the
// list still bounds how many there can be, so this cannot bring back the
// termination problem that keeps `while` out as a statement.

/// An effect that counts turns, so "did it stop" is a number rather than a
/// feeling.
const COUNTING: &str = "module a\n\n\
     effect Log {\n\
     \x20 fn note() -> ()\n\
     \x20 fn seen() -> Int\n\
     }\n\n\
     handler Loud implements Log {\n\
     \x20 state count: Int\n\n\
     \x20 fn note() -> () {\n\
     \x20   count = count + 1\n\
     \x20 }\n\n\
     \x20 fn seen() -> Int {\n\
     \x20   count\n\
     \x20 }\n\
     }\n\n";

#[test]
fn a_turn_the_condition_refuses_is_not_taken() {
    // The measurement. Both of these answer `true` on the third element. The
    // one that cannot stop takes a fourth turn to find that out, and a branch
    // in the body can skip the work but not the turn.
    expect_pass(&format!(
        "{COUNTING}\
         fn stopping(items: List<Int>) -> Bool\n\
         \x20 uses Log.note,\n\
         {{\n\
         \x20 for item in items with found = false while !found {{\n\
         \x20   Log.note()\n\
         \x20   item > 2\n\
         \x20 }}\n\
         }}\n\n\
         fn walking(items: List<Int>) -> Bool\n\
         \x20 uses Log.note,\n\
         {{\n\
         \x20 for item in items with found = false {{\n\
         \x20   Log.note()\n\
         \x20   if found {{ true }} else {{ item > 2 }}\n\
         \x20 }}\n\
         }}\n\n\
         test \"a turn not taken\" {{\n\
         \x20 with Loud {{ count: 0 }} {{\n\
         \x20   assert stopping([1, 2, 3, 4]) == true\n\
         \x20   assert Log.seen() == 3\n\
         \x20 }}\n\n\
         \x20 with Loud {{ count: 0 }} {{\n\
         \x20   assert walking([1, 2, 3, 4]) == true\n\
         \x20   assert Log.seen() == 4\n\
         \x20 }}\n\
         }}\n"
    ));
}

#[test]
fn a_condition_that_is_false_from_the_start_takes_no_turns_at_all() {
    expect_pass(
        "module a\n\n\
         fn none(items: List<Int>) -> Int {\n\
         \x20 for item in items with total = 0 while total > 0 {\n\
         \x20   total + item\n\
         \x20 }\n\
         }\n\n\
         test \"the accumulator comes back untouched\" {\n\
         \x20 assert none([1, 2, 3]) == 0\n\
         }\n",
    );
}

#[test]
fn the_condition_has_to_be_a_bool() {
    let (_, checked) = check(
        "module a\n\n\
         fn f(items: List<Int>) -> Int {\n\
         \x20 for item in items with total = 0 while total {\n\
         \x20   total + item\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::TYPE_MISMATCH]
    );
}

/// The condition is about what the walk has worked out so far. Without an
/// accumulator there is nothing that changes between turns, so it either stops
/// the walk before it starts or never stops it, and neither is a thing anybody
/// meant to write.
#[test]
fn a_condition_with_nothing_to_be_about_is_refused() {
    let (sources, checked) = check(
        "module a\n\n\
         fn f(items: List<Int>) -> () {\n\
         \x20 for item in items while true {\n\
         \x20   ()\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::WHILE_WITHOUT_ACCUMULATOR]
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("needs a `with`"), "{text}");
}

/// The element belongs to the turn this is deciding whether to take, so it is
/// not in scope yet. Nor is the index.
#[test]
fn the_element_is_not_in_scope_in_the_condition() {
    let (_, checked) = check(
        "module a\n\n\
         fn f(items: List<Int>) -> Int {\n\
         \x20 for item at here in items with total = 0 while item > 0 {\n\
         \x20   total + item\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_resolve::codes::UNKNOWN_NAME]
    );
}

/// `while` is a name everywhere else, and the only thing that can come between
/// an accumulator and the body is this one, so there is nothing to reserve.
/// Same reasoning that kept `at` and `state` out of the keyword list.
#[test]
fn while_is_still_a_name() {
    expect_pass(
        "module a\n\n\
         fn wait(while: Int) -> Int {\n\
         \x20 while + 1\n\
         }\n\n\
         test \"a name is a name\" {\n\
         \x20 assert wait(1) == 2\n\
         }\n",
    );
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
