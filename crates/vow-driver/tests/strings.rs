//! `split`, `join`, `to_string` and `to_int`.
//!
//! Two pairs of inverses, and the reason they exist is that before them a
//! program could hold text and hold a number and get from neither to the
//! other. Nothing could take input apart, put output together, or print a
//! count.

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
    );
    let outcomes = run_tests(&program, checked.file);
    (sources, outcomes)
}

/// Runs a body as the one test in a file, which is expected to pass.
fn expect_pass(body: &str) {
    let src = format!("module a\n\ntest \"t\" {{\n{body}\n}}\n");
    let (sources, outcomes) = run(&src);
    assert_eq!(outcomes.len(), 1, "expected exactly one test");
    if let Some(failure) = &outcomes[0].failure {
        panic!("should have passed:\n{}", render_human(&sources, failure));
    }
}

// -- types ------------------------------------------------------------------

#[test]
fn the_pairs_line_up() {
    check_ok(
        "module a\n\n\
         fn f(line: String) -> String { join(split(line, \",\"), \";\") }\n\n\
         fn g(n: Int) -> Result<Int, String> { to_int(to_string(n)) }\n",
    );
}

#[test]
fn split_gives_a_list_of_strings_and_nothing_else() {
    let (_, checked) =
        check("module a\n\nfn f(line: String) -> List<Int> { split(line, \",\") }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn join_will_not_take_a_list_of_numbers() {
    let (_, checked) = check("module a\n\nfn f(ns: List<Int>) -> String { join(ns, \",\") }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn an_empty_list_can_be_joined() {
    // `[]` is a `List<unknown>`, and unknown agrees with the `String` this
    // wants, which is the whole reason the empty literal is usable at all.
    check_ok("module a\n\nfn f() -> String { join([], \",\") }\n");
}

#[test]
fn to_int_can_fail_and_says_so_in_its_type() {
    // The failure is in the type rather than in a trap, so a caller that
    // ignores it does not type check.
    let (_, checked) = check("module a\n\nfn f(text: String) -> Int { to_int(text) }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn to_string_will_not_take_a_string() {
    let (_, checked) = check("module a\n\nfn f(text: String) -> String { to_string(text) }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::TYPE_MISMATCH]
    );
}

// -- running ----------------------------------------------------------------

#[test]
fn splitting_and_joining_are_inverses() {
    expect_pass(
        "\x20 assert split(\"a,b,c\", \",\") == [\"a\", \"b\", \"c\"]\n\
         \x20 assert join([\"a\", \"b\", \"c\"], \",\") == \"a,b,c\"\n\
         \x20 assert join(split(\"a,b,c\", \",\"), \",\") == \"a,b,c\"",
    );
}

#[test]
fn splitting_something_with_no_separator_in_it_gives_one_piece() {
    expect_pass(
        "\x20 assert split(\"abc\", \",\") == [\"abc\"]\n\
         \x20 assert split(\"\", \",\") == [\"\"]\n\
         \x20 assert join([], \",\") == \"\"\n\
         \x20 assert join([\"only\"], \",\") == \"only\"",
    );
}

#[test]
fn an_empty_separator_gives_the_characters() {
    // Characters, not bytes, the same as `length`. Otherwise the two would
    // disagree about what a string is made of.
    expect_pass(
        "\x20 assert split(\"gün\", \"\") == [\"g\", \"ü\", \"n\"]\n\
         \x20 assert length(split(\"gün\", \"\")) == length(\"gün\")\n\
         \x20 assert split(\"\", \"\") == []",
    );
}

#[test]
fn a_separator_at_the_edges_leaves_empty_pieces() {
    // Dropping them would make `split` and `join` stop being inverses, which
    // is the only property either of them has.
    expect_pass(
        "\x20 assert split(\",a,\", \",\") == [\"\", \"a\", \"\"]\n\
         \x20 assert join(split(\",a,\", \",\"), \",\") == \",a,\"",
    );
}

#[test]
fn numbers_go_to_text_and_back() {
    expect_pass(
        "\x20 assert to_string(0) == \"0\"\n\
         \x20 assert to_string(0 - 12) == \"-12\"\n\
         \x20 assert to_int(\"41\") == ok(41)\n\
         \x20 assert to_int(\"-41\") == ok(0 - 41)\n\
         \x20 assert to_int(to_string(7)) == ok(7)",
    );
}

#[test]
fn text_that_is_not_a_number_comes_back_as_an_error() {
    expect_pass(
        "\x20 assert to_int(\"\") == err(\"`` is not a number\")\n\
         \x20 assert to_int(\" 4\") == err(\"` 4` is not a number\")\n\
         \x20 assert to_int(\"4.5\") == err(\"`4.5` is not a number\")\n\
         \x20 assert to_int(\"99999999999999999999\") == err(\"`99999999999999999999` is not a number\")",
    );
}
