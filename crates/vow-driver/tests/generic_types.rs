//! Generic records and choices.
//!
//! `Result` and `List` are built into the language because there was no way to
//! declare either. That was always written down as a shortcut, with the note
//! that a third would be the point where it had clearly stopped paying.
//! `Option` is the third, and it is declarable now.
//!
//! A generic nominal type is a head plus arguments, and two of them are the
//! same type only when the head matches and the arguments match componentwise.
//! That is exactly how `Result` and `List` were already compared, which is why
//! this cost so little.
//!
//! What decides the arguments is the same matching #112 does for a call: a
//! literal matches the declared field types against the values it was given.

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::{Checked, check_all, check_text};
use vow_interp::{Program, run_tests};

fn check(src: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.vow", src.to_string());
    (sources, checked)
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

fn expect_clean(src: &str) -> (SourceMap, Checked) {
    let (sources, checked) = check(src);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    (sources, checked)
}

fn expect_refused(src: &str) -> String {
    let (sources, checked) = check(src);
    assert!(
        checked.has_errors(),
        "this should not have been accepted:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
    rendered(&sources, &checked.diagnostics)
}

fn expect_tests_pass(src: &str) {
    let (sources, checked) = expect_clean(src);
    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        checked.guards(),
    );
    for outcome in run_tests(&program, checked.file) {
        if let Some(failure) = &outcome.failure {
            panic!(
                "`{}` should have passed:\n{}",
                outcome.name,
                render_human(&sources, failure)
            );
        }
    }
}

const PAIR: &str = "module a\n\n\
     record Pair<A, B> {\n\
     \x20   left: A,\n\
     \x20   right: B,\n\
     }\n\n";

const OPTION: &str = "module a\n\n\
     choice Option<T> {\n\
     \x20   None,\n\
     \x20   Some { value: T },\n\
     }\n\n";

// -- what a literal works out --------------------------------------------------

#[test]
fn a_literal_says_what_its_arguments_are() {
    expect_clean(&format!(
        "{PAIR}\
         fn f() -> Pair<Int, String> {{ Pair {{ left: 1, right: \"a\" }} }}\n"
    ));
}

#[test]
fn a_literal_that_says_something_else_is_refused() {
    let text = expect_refused(&format!(
        "{PAIR}\
         fn f() -> Pair<Int, String> {{ Pair {{ left: 1, right: 2 }} }}\n"
    ));
    assert!(text.contains("VOW4001"), "{text}");
    assert!(text.contains("Pair<Int, Int>"), "{text}");
}

#[test]
fn a_field_reads_at_the_type_it_was_applied_to() {
    // `left` on a `Pair<Int, String>` is an `Int` rather than the `A` the
    // declaration wrote. Without the substitution this would be unknown, and
    // unknown agrees with everything.
    let text = expect_refused(&format!(
        "{PAIR}\
         fn f(p: Pair<Int, String>) -> String {{ p.left }}\n"
    ));
    assert!(text.contains("VOW4001"), "{text}");
    assert!(text.contains("expected `String`, found `Int`"), "{text}");
}

#[test]
fn the_head_has_to_match_and_not_just_the_shape() {
    let text = expect_refused(
        "module a\n\n\
         record Pair<A, B> { left: A, right: B }\n\n\
         record Other<A, B> { left: A, right: B }\n\n\
         fn f(p: Pair<Int, Int>) -> Other<Int, Int> { p }\n",
    );
    assert!(text.contains("VOW4001"), "{text}");
}

#[test]
fn a_variant_works_its_argument_out_from_its_payload() {
    expect_tests_pass(&format!(
        "{OPTION}\
         fn or_else<T>(option: Option<T>, fallback: T) -> T {{\n\
         \x20 match option {{\n\
         \x20   None => fallback,\n\
         \x20   Some {{ value }} => value,\n\
         \x20 }}\n\
         }}\n\n\
         test \"it works\" {{\n\
         \x20 assert or_else(Some {{ value: 7 }}, 0) == 7\n\
         \x20 assert or_else(Some {{ value: \"a\" }}, \"b\") == \"a\"\n\
         }}\n"
    ));
}

#[test]
fn a_bare_variant_says_nothing_and_agrees_with_everything() {
    // The same answer `[]` gets for its element type and `ok(x)` gets for its
    // error type. A third answer to the same question would be a third thing
    // to explain.
    expect_tests_pass(&format!(
        "{OPTION}\
         fn or_else<T>(option: Option<T>, fallback: T) -> T {{\n\
         \x20 match option {{\n\
         \x20   None => fallback,\n\
         \x20   Some {{ value }} => value,\n\
         \x20 }}\n\
         }}\n\n\
         test \"it works\" {{\n\
         \x20 assert or_else(None, 5) == 5\n\
         \x20 assert or_else(None, \"none\") == \"none\"\n\
         }}\n"
    ));
}

#[test]
fn a_pattern_binder_reads_at_the_type_the_scrutinee_was_applied_to() {
    let text = expect_refused(&format!(
        "{OPTION}\
         fn f(option: Option<Int>) -> String {{\n\
         \x20 match option {{\n\
         \x20   None => \"none\",\n\
         \x20   Some {{ value }} => value,\n\
         \x20 }}\n\
         }}\n"
    ));
    assert!(text.contains("VOW4001"), "{text}");
    assert!(text.contains("expected `String`, found `Int`"), "{text}");
}

// -- what a type has to be written as ------------------------------------------

#[test]
fn a_generic_type_takes_exactly_what_it_declared() {
    // In both directions, because a signature is complete. `Pair` written
    // bare is as much a hole in one as a parameter with no type, and filling
    // it in with unknowns would make every use of it agree with everything.
    let text = expect_refused(&format!("{PAIR}fn f(p: Pair) -> Int {{ 0 }}\n"));
    assert!(text.contains("VOW4013"), "{text}");
    assert!(text.contains("`Pair` takes 2 type arguments"), "{text}");

    let text = expect_refused(&format!("{PAIR}fn f(p: Pair<Int>) -> Int {{ 0 }}\n"));
    assert!(text.contains("and 1 was given"), "{text}");

    let text = expect_refused(&format!(
        "{PAIR}fn f(p: Pair<Int, String, Bool>) -> Int {{ 0 }}\n"
    ));
    assert!(text.contains("and 3 were given"), "{text}");
}

#[test]
fn a_type_declared_with_no_parameters_still_takes_none() {
    let text = expect_refused("module a\n\nrecord R { n: Int }\n\nfn f(x: R<Int>) -> Int { 0 }\n");
    assert!(text.contains("`R` takes no type arguments"), "{text}");
}

#[test]
fn a_type_parameter_is_not_a_free_pass_inside_a_declaration() {
    // `A` is whatever the use decided, so a field of that type holds one and
    // nothing more is known about it.
    let text = expect_refused(&format!(
        "{PAIR}\
         fn f(p: Pair<Int, String>) -> Int {{ p.right }}\n"
    ));
    assert!(text.contains("expected `Int`, found `String`"), "{text}");
}

// -- across a module boundary ---------------------------------------------------

#[test]
fn a_generic_type_crosses_a_module_boundary() {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = [
        "module app\n\n\
         use lib.{Box, hold}\n\n\
         fn f() -> Box<Int> { hold(1) }\n\n\
         fn g(b: Box<String>) -> String { b.value }\n",
        "module lib\n\n\
         record Box<T> { value: T }\n\n\
         fn hold<T>(value: T) -> Box<T> { Box { value } }\n",
    ]
    .iter()
    .enumerate()
    .map(|(index, text)| sources.add(format!("file{index}.vow"), *text))
    .collect();

    let mut checks = check_all(&sources, &ids);
    let checked = checks.remove(0);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn an_imported_generic_type_is_checked_at_the_arguments_it_was_given() {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = [
        "module app\n\n\
         use lib.{Box}\n\n\
         fn g(b: Box<String>) -> Int { b.value }\n",
        "module lib\n\n\
         record Box<T> { value: T }\n",
    ]
    .iter()
    .enumerate()
    .map(|(index, text)| sources.add(format!("file{index}.vow"), *text))
    .collect();

    let mut checks = check_all(&sources, &ids);
    let checked = checks.remove(0);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(checked.has_errors(), "this should not have been accepted");
    assert!(text.contains("expected `Int`, found `String`"), "{text}");
}
