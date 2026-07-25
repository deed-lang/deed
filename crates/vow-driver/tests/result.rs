//! `Result`, `ok`, `err` and `?`, which the language provides rather than a
//! library.
//!
//! The point of the change was that a function could not fail without an
//! import, so most of these are about failing.

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::{check_all, check_text};

fn check(src: &str) -> (SourceMap, vow_driver::Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.vow", src);
    (sources, checked)
}

/// Checks `src` with `deps` compiled alongside it.
///
/// An import with nothing behind it is an error now, so any test that touches
/// one needs a real module on the other side of it.
fn check_with(src: &str, deps: &[&str]) -> (SourceMap, vow_driver::Checked) {
    let mut sources = SourceMap::new();
    let mut files = vec![sources.add("test.vow", src)];
    for (index, dep) in deps.iter().enumerate() {
        files.push(sources.add(format!("dep{index}.vow"), *dep));
    }
    let checked = check_all(&sources, &files).remove(0);
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

const ERRORS: &str = "\
choice Failure {
    TooBig { limit: Int },
    Empty,
}
";

fn with_errors(rest: &str) -> String {
    format!("module a\n\n{ERRORS}\n{rest}")
}

// -- the constructors ------------------------------------------------------

#[test]
fn a_function_can_fail_without_importing_anything() {
    check_ok(&with_errors(
        "fn small(n: Int) -> Result<Int, Failure> {\n\
         \x20 if n > 10 {\n\
         \x20   return err(TooBig { limit: 10 })\n\
         \x20 }\n\
         \x20 ok(n)\n\
         }\n",
    ));
}

#[test]
fn ok_says_nothing_about_the_error_type_and_vice_versa() {
    // This is what replaces unification. `ok(x)` is `Result<T, unknown>`, and
    // unknown agrees with whatever the expected error type turns out to be.
    check_ok(&with_errors(
        "fn one() -> Result<Int, Failure> { ok(1) }\n\n\
         fn two() -> Result<Bool, Failure> { err(Empty) }\n\n\
         fn three() -> Result<Int, Int> { ok(1) }\n",
    ));
}

#[test]
fn the_value_type_is_still_checked() {
    let (sources, checked) = check(&with_errors(
        "fn f() -> Result<Int, Failure> { ok(true) }\n",
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::TYPE_MISMATCH]
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("expected `Result<Int, Failure>`, found `Result<Bool, _>`"),
        "{text}"
    );
}

#[test]
fn the_error_type_is_still_checked() {
    let (sources, checked) = check(&with_errors("fn f() -> Result<Int, Failure> { err(1) }\n"));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::TYPE_MISMATCH]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("`Result<_, Int>`"));
}

#[test]
fn result_needs_exactly_two_type_arguments() {
    let (sources, checked) = check("module a\n\nfn f() -> Result<Int> { ok(1) }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::NOT_GENERIC]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("`Result<Value, Error>`"));
}

// -- the question mark -----------------------------------------------------

#[test]
fn the_question_mark_unwraps_the_success_case() {
    check_ok(&with_errors(
        "fn inner() -> Result<Int, Failure> { ok(1) }\n\n\
         fn outer() -> Result<Int, Failure> {\n\
         \x20 let n = inner()?\n\
         \x20 ok(n + 1)\n\
         }\n",
    ));
}

#[test]
fn the_question_mark_needs_a_result() {
    let (sources, checked) =
        check("module a\n\nfn f() -> Result<Int, Int> {\n  let n = 1?\n  ok(n)\n}\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::NOT_A_RESULT]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("`?` needs a `Result`"));
}

#[test]
fn the_question_mark_needs_somewhere_to_propagate_to() {
    let (sources, checked) = check(&with_errors(
        "fn inner() -> Result<Int, Failure> { ok(1) }\n\n\
         fn outer() -> Int {\n\
         \x20 inner()?\n\
         }\n",
    ));
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::TRY_NEEDS_RESULT_RETURN),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("nowhere to propagate"));
}

#[test]
fn the_error_types_have_to_line_up() {
    let (sources, checked) = check(&with_errors(
        "fn inner() -> Result<Int, Failure> { ok(1) }\n\n\
         fn outer() -> Result<Int, Int> {\n\
         \x20 let n = inner()?\n\
         \x20 ok(n)\n\
         }\n",
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::TYPE_MISMATCH]
    );
    assert!(
        rendered(&sources, &checked.diagnostics).contains("the error type this function returns")
    );
}

// -- the prelude -----------------------------------------------------------

#[test]
fn importing_a_name_the_language_provides_warns() {
    // Silently shadowing the builtin would put everything that depends on it
    // quietly back to being unchecked.
    let (sources, checked) = check_with(
        "module a\n\nuse std/result.{Result}\n\nfn f() -> Result<Int, Int> { ok(1) }\n",
        &["module std/result\n\nrecord Result { n: Int }\n"],
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_resolve::codes::SHADOWED_DECLARATION),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert!(!checked.has_errors(), "a warning, not a rejection");
    assert!(rendered(&sources, &checked.diagnostics).contains("stops being checked"));
}

#[test]
fn result_has_no_methods() {
    // Found in examples/transfer.vow the moment `Result` stopped being unknown.
    let (sources, checked) = check(&with_errors(
        "fn f() -> Result<Int, Failure> { ok(1) }\n\n\
         fn g() -> Int { f().unwrap() }\n",
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::NO_SUCH_FIELD]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("has no field `unwrap`"));
}
