//! `split`, `join`, `trim`, `to_string` and `to_int`.
//!
//! Two pairs of inverses, and the reason they exist is that before them a
//! program could hold text and hold a number and get from neither to the
//! other. Nothing could take input apart, put output together, or print a
//! count.
//!
//! `trim` is the odd one out and is here for a different reason: it is the one
//! text operation that cannot be written in the language. `contains` is
//! `length(split(a, b)) > 1` and `replace` is `join(split(a, from), to)`, but
//! deciding what whitespace is needs to look at characters and taking it off
//! the ends needs a walk that stops early, which a fold does not do.

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
        vec![deed_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn join_will_not_take_a_list_of_numbers() {
    let (_, checked) = check("module a\n\nfn f(ns: List<Int>) -> String { join(ns, \",\") }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::TYPE_MISMATCH]
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
        vec![deed_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn to_string_will_not_take_a_string() {
    let (_, checked) = check("module a\n\nfn f(text: String) -> String { to_string(text) }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn trim_takes_text_and_gives_text() {
    check_ok("module a\n\nfn f(text: String) -> String { trim(text) }\n");

    let (_, checked) = check("module a\n\nfn f(n: Int) -> String { trim(n) }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::TYPE_MISMATCH]
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

#[test]
fn trim_takes_whitespace_off_both_ends_and_nowhere_else() {
    expect_pass(
        "\x20 assert trim(\"  a  \") == \"a\"\n\
         \x20 assert trim(\"a b\") == \"a b\"\n\
         \x20 assert trim(\"  a  b  \") == \"a  b\"\n\
         \x20 assert trim(\"a\") == \"a\"",
    );
}

#[test]
fn whitespace_is_four_characters_and_they_are_written_down() {
    // Space, tab, carriage return and newline, rather than the Unicode
    // whitespace table. A four letter name should not stand for a table nobody
    // reading the signature can see.
    expect_pass(
        "\x20 assert trim(\" \\t\\r\\na\\n\\r\\t \") == \"a\"\n\
         \x20 assert trim(\"\") == \"\"\n\
         \x20 assert trim(\"   \") == \"\"",
    );
}

#[test]
fn trim_is_what_makes_a_line_from_a_windows_file_a_line() {
    // The reason this exists. Splitting on a newline leaves the carriage
    // return on every piece, and before there was a way to take it off, the
    // only thing stopping the example programs from printing over themselves
    // was the repository forcing LF.
    expect_pass(
        "\x20 assert split(\"a\\r\\nb\\r\\n\", \"\\n\") == [\"a\\r\", \"b\\r\", \"\"]\n\
         \x20 assert trim(\"a\\r\") == \"a\"",
    );
}

#[test]
fn what_trim_is_not_for() {
    // Everything else people reach for here can already be written, which is
    // the test for whether a name belongs in the prelude.
    expect_pass(
        "\x20 assert length(split(\"hay needle hay\", \"needle\")) > 1\n\
         \x20 assert length(split(\"hay hay\", \"needle\")) == 1\n\
         \x20 assert join(split(\"a-b-c\", \"-\"), \"+\") == \"a+b+c\"",
    );
}

// -- case ------------------------------------------------------------------

#[test]
fn upper_and_lower_take_text_and_give_text() {
    check_ok("module a\n\nfn f(text: String) -> String { upper(text) }\n");
    check_ok("module a\n\nfn f(text: String) -> String { lower(text) }\n");

    let (_, checked) = check("module a\n\nfn f(n: Int) -> String { upper(n) }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        [deed_typeck::codes::TYPE_MISMATCH]
    );
}

#[test]
fn case_is_the_twenty_six_letters_and_nothing_else() {
    // The same bargain `trim` makes about whitespace. Deciding what an
    // uppercase letter is in general needs a table, and a name this short may
    // not depend on one a reader of the signature cannot see.
    expect_pass(
        "\x20 assert upper(\"deed lang 1\") == \"DEED LANG 1\"\n\
         \x20 assert lower(\"DEED Lang 1\") == \"deed lang 1\"",
    );
}

#[test]
fn a_character_outside_the_alphabet_comes_back_as_it_went_in() {
    // Text in a script with no case survives rather than being mangled by a
    // rule that was not written for it, and a digit is not a letter.
    expect_pass(
        "\x20 assert upper(\"\u{fc}n\u{ef}code\") == \"\u{fc}N\u{ef}CODE\"\n\
         \x20 assert lower(\"\u{dc}N\u{cf}CODE\") == \"\u{dc}n\u{cf}code\"\n\
         \x20 assert upper(\"1-2 3\") == \"1-2 3\"",
    );
}

#[test]
fn case_of_nothing_is_nothing() {
    // The empty string is the one input every text operation here has to
    // answer for, and it is the one that used to fall off the end.
    expect_pass(
        "\x20 assert upper(\"\") == \"\"\n\
         \x20 assert lower(\"\") == \"\"",
    );
}

#[test]
fn letters_have_no_numeric_code_in_the_language() {
    // A program can reach the characters of a string with `split(s, "")`, and
    // `std/string` can write a finite case table over those one-character
    // strings, but there is still nothing that turns an arbitrary character
    // into a number. `to_int` speaks about text that spells a number, so it
    // covers the ten digits and no letter.
    let (sources, checked) = check(
        "module a\n\nfn code_of(letter: String) -> Result<Int, String> {\n    to_int(letter)\n}\n\n\
         test \"a letter is not a number\" {\n    assert code_of(\"a\") == err(\"`a` is not a number\")\n}\n",
    );
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    let (_, outcomes) = run(
        "module a\n\ntest \"a letter is not a number\" {\n\x20   assert to_int(\"a\") == err(\"`a` is not a number\")\n}\n",
    );
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].failure.is_none());
}
