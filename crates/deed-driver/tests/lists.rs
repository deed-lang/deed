//! `List`, `at` and `push`, which the language provides rather than a library.
//!
//! There is still no way to declare a generic type, so this is `Result`'s
//! trick applied a second time: the element type is compared componentwise and
//! an unknown one absorbs. Most of these are about what that buys and where it
//! stops.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::check_text;
use deed_interp::{Program, TestOutcome, run_tests};

fn check(src: &str) -> (SourceMap, deed_driver::Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.deed", src);
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

/// Checks and runs a source, which must check cleanly.
fn run(src: &str) -> (SourceMap, Vec<TestOutcome>) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.deed", src.to_string());
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

/// Runs a source whose tests are all expected to pass.
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

// -- literals --------------------------------------------------------------

#[test]
fn a_literal_takes_its_element_type_from_the_first_element() {
    check_ok("module a\n\nfn f() -> List<Int> { [1, 2, 3] }\n");
    check_ok("module a\n\nfn f() -> List<String> { [\"a\"] }\n");
}

#[test]
fn the_elements_have_to_agree() {
    let (_, checked) = check("module a\n\nfn f() -> List<Int> { [1, \"two\"] }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn only_the_first_disagreement_is_reported_per_element() {
    // Three elements, two of them wrong, two diagnostics. Not one about the
    // list as a whole, because the list is not the mistake.
    let (_, checked) = check("module a\n\nfn f() -> List<Int> { [1, \"two\", true] }\n");
    assert_eq!(checked.diagnostics.len(), 2);
}

#[test]
fn an_empty_literal_fits_where_any_list_was_wanted() {
    // The same absorbing that lets `ok(x)` say nothing about the error type.
    // Without it there would be no way to write an empty list at all.
    check_ok("module a\n\nfn f() -> List<Int> { [] }\n");
    check_ok("module a\n\nfn f() -> List<String> { [] }\n");
    check_ok(
        "module a\n\n\
         fn take(items: List<Int>) -> Int { length(items) }\n\n\
         fn f() -> Int { take([]) }\n",
    );
}

#[test]
fn a_list_of_lists_is_a_list() {
    check_ok("module a\n\nfn f() -> List<List<Int>> { [[1], [], [2, 3]] }\n");
}

#[test]
fn a_list_of_one_type_is_not_a_list_of_another() {
    let (_, checked) = check(
        "module a\n\n\
         fn take(items: List<Int>) -> Int { length(items) }\n\n\
         fn f() -> Int { take([\"a\"]) }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::TYPE_MISMATCH]
    );
}

// -- the type ---------------------------------------------------------------

#[test]
fn list_needs_exactly_one_type_argument() {
    let (_, checked) = check("module a\n\nfn f(items: List<Int, String>) -> Int { 0 }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::NOT_GENERIC]
    );

    let (_, checked) = check("module a\n\nfn f(items: List) -> Int { 0 }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::NOT_GENERIC]
    );
}

#[test]
fn a_list_crosses_a_module_boundary() {
    // The surface lowering is a second copy of the type lowering, and a type
    // that only one of them knows about is a type that becomes unknown on the
    // way out of the module. Two of those have shipped before.
    let mut sources = SourceMap::new();
    let dep = sources.add(
        "dep.deed",
        "module dep\n\nfn digits() -> List<Int> { [1, 2] }\n".to_string(),
    );
    let main = sources.add(
        "test.deed",
        "module a\n\nuse dep.{digits}\n\nfn f() -> Int { length(digits()) }\n".to_string(),
    );
    let checked = deed_driver::check_all(&sources, &[main, dep]).remove(0);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- the operations ---------------------------------------------------------

#[test]
fn length_measures_a_list_as_well_as_a_string() {
    check_ok("module a\n\nfn f(items: List<Bool>) -> Int { length(items) }\n");
}

#[test]
fn a_list_length_is_never_negative() {
    // Same promise `length` makes about a string, and it has to be the same
    // promise or a refinement over one would become a runtime check.
    check_ok(
        "module a\n\n\
         type Counted = Int where value >= 0\n\n\
         fn f(items: List<Int>) -> Counted { length(items) }\n",
    );
}

#[test]
fn at_hands_back_a_result() {
    check_ok(
        "module a\n\n\
         fn f(items: List<String>) -> Result<String, String> { at(items, 0) }\n",
    );
}

#[test]
fn at_wants_a_number_for_an_index() {
    let (_, checked) = check(
        "module a\n\n\
         fn f(items: List<String>) -> Result<String, String> { at(items, \"0\") }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn push_hands_back_a_list_of_the_same_thing() {
    check_ok(
        "module a\n\n\
         fn f(items: List<Int>) -> List<Int> { push(items, 4) }\n",
    );
}

#[test]
fn push_will_not_mix_element_types() {
    let (_, checked) = check(
        "module a\n\n\
         fn f(items: List<Int>) -> List<Int> { push(items, \"four\") }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn the_list_operations_refuse_things_that_are_not_lists() {
    for src in [
        "module a\n\nfn f(n: Int) -> Int { length(n) }\n",
        "module a\n\nfn f(n: Int) -> Result<Int, String> { at(n, 0) }\n",
        "module a\n\nfn f(s: String) -> Int { length(push(s, 1)) }\n",
    ] {
        let (sources, checked) = check(src);
        assert!(
            codes_of(&checked.diagnostics).contains(&deed_typeck::codes::NOT_A_LIST),
            "{}",
            rendered(&sources, &checked.diagnostics)
        );
    }
}

#[test]
fn the_list_operations_count_their_arguments() {
    for src in [
        "module a\n\nfn f(items: List<Int>) -> Int { length(items, 1) }\n",
        "module a\n\nfn f(items: List<Int>) -> List<Int> { push(items) }\n",
    ] {
        let (sources, checked) = check(src);
        assert_eq!(
            codes_of(&checked.diagnostics),
            vec![deed_typeck::codes::WRONG_ARITY],
            "{}",
            rendered(&sources, &checked.diagnostics)
        );
    }
}

// -- running ----------------------------------------------------------------

#[test]
fn lists_compare_by_their_elements() {
    expect_pass(
        "module a\n\n\
         test \"structural\" {\n\
         \x20 assert [1, 2] == [1, 2]\n\
         \x20 assert [1, 2] != [2, 1]\n\
         \x20 assert [] == []\n\
         }\n",
    );
}

#[test]
fn push_leaves_the_list_it_was_given_alone() {
    // Handler state is meant to be the only mutable thing in the language, so
    // a collection that could be written through would quietly be a second
    // one.
    expect_pass(
        "module a\n\n\
         test \"immutable\" {\n\
         \x20 let start = [1]\n\
         \x20 let longer = push(start, 2)\n\
         \x20 assert start == [1]\n\
         \x20 assert longer == [1, 2]\n\
         }\n",
    );
}

#[test]
fn reading_past_the_end_is_a_value_rather_than_a_stop() {
    expect_pass(
        "module a\n\n\
         test \"out of range\" {\n\
         \x20 assert at([7, 8], 1) == ok(8)\n\
         \x20 assert at([7, 8], 2) == err(\"index 2 is outside a list of 2\")\n\
         \x20 assert at([7, 8], 0 - 1) == err(\"index -1 is outside a list of 2\")\n\
         }\n",
    );
}

#[test]
fn a_list_can_be_walked_with_recursion() {
    // There is no loop syntax, so this is the only way, and it costs a
    // `Diverge` in the row every time. That cost is the argument for deciding
    // what iteration looks like rather than leaving it out forever.
    expect_pass(
        "module a\n\n\
         fn sum_from(numbers: List<Int>, index: Int) -> Int\n\
         \x20 uses\n\
         \x20   Diverge,\n\
         {\n\
         \x20 match at(numbers, index) {\n\
         \x20   ok(n) => n + sum_from(numbers, index + 1),\n\
         \x20   err(why) => 0,\n\
         \x20 }\n\
         }\n\n\
         test \"walking\" {\n\
         \x20 assert sum_from([1, 2, 3], 0) == 6\n\
         \x20 assert sum_from([], 0) == 0\n\
         }\n",
    );
}

// -- repeat ------------------------------------------------------------------
//
// A `for` walks a list that already exists, so having something a number of
// times had no list to hand it and the only form left was a function calling
// itself. That made padding a column declare `Diverge` and spread it to
// everything that built a line, which is the outcome the design document gives
// as the reason iteration exists in the first place.

#[test]
fn padding_a_column_needs_no_effects_at_all() {
    // The function this exists for. Written with recursion it declares
    // `Diverge`, and so does everything that calls it.
    expect_pass(
        "module a\n\n\
         fn padded(text: String, width: Int) -> String {\n\
         \x20 text + join(repeat(\" \", width - length(text)), \"\")\n\
         }\n\n\
         test \"padding\" {\n\
         \x20 assert length(padded(\"ab\", 20)) == 20\n\
         \x20 assert padded(\"ab\", 4) == \"ab  \"\n\
         }\n",
    );
}

#[test]
fn a_count_that_went_negative_is_no_repetitions() {
    // Not a refusal. `repeat(\" \", width - length(text))` goes negative
    // exactly when the text is already wider than the column, and what it
    // means there is that there is no padding to add.
    expect_pass(
        "module a\n\n\
         test \"nothing to repeat\" {\n\
         \x20 assert repeat(\"x\", 0) == []\n\
         \x20 assert repeat(\"x\", 0 - 4) == []\n\
         }\n",
    );
}

/// The count comes back out through `at`, which already binds a position and
/// already knows it is not negative and below the length. So counting from
/// zero needed no second thing to walk.
#[test]
fn counting_from_zero_comes_back_through_at() {
    expect_pass(
        "module a\n\n\
         fn upto(count: Int) -> List<Int> {\n\
         \x20 for _step at i in repeat(0, count) with out = [] {\n\
         \x20   push(out, i)\n\
         \x20 }\n\
         }\n\n\
         test \"counting\" {\n\
         \x20 assert upto(4) == [0, 1, 2, 3]\n\
         \x20 assert upto(0) == []\n\
         }\n",
    );
}

#[test]
fn it_repeats_whatever_it_was_given() {
    expect_pass(
        "module a\n\n\
         record Point { x: Int }\n\n\
         test \"any element type\" {\n\
         \x20 assert repeat(true, 2) == [true, true]\n\
         \x20 assert repeat([1], 2) == [[1], [1]]\n\
         \x20 assert repeat(Point { x: 1 }, 2) == [Point { x: 1 }, Point { x: 1 }]\n\
         }\n",
    );
}

#[test]
fn the_count_has_to_be_a_number() {
    let (sources, checked) = check(
        "module a\n\n\
         fn f() -> List<String> { repeat(\"x\", \"three\") }\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("DEED4001"), "{text}");
}

#[test]
fn the_element_type_is_the_type_of_what_was_repeated() {
    // A list of the wrong thing is refused, which is what says the element
    // type was worked out rather than left unknown.
    let (sources, checked) = check(
        "module a\n\n\
         fn f() -> List<Int> { repeat(\"x\", 3) }\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("DEED4001"), "{text}");
}
