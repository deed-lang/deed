//! Function values, and the promise a function type makes.
//!
//! `Fn(Int) -> Int` says two things: this takes an `Int` and hands back an
//! `Int`, and it performs no effects. The second is not decoration. There is no
//! syntax for a row on a function type, and leaving one off cannot mean "any
//! row": a value that carried an unstated effect through a signature would undo
//! the point of having rows at all.
//!
//! These run the whole pipeline, and they have to. Deciding which values have
//! to keep the promise is a question about types and deciding whether they do
//! is a question about rows, so no single pass can answer it.

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::{Checked, check_text};
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

/// Asserts the source is rejected, and hands back what was said about it.
fn expect_refused(src: &str) -> String {
    let (sources, checked) = check(src);
    assert!(
        checked.has_errors(),
        "this should not have been accepted:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
    rendered(&sources, &checked.diagnostics)
}

const LOG: &str = "module a\n\n\
     effect Log {\n    fn note(message: String) -> ()\n}\n\n\
     handler Counted implements Log {\n\
     \x20 state seen: Int\n\n\
     \x20 fn note(message) -> () {\n\
     \x20   seen = seen + 1\n\
     \x20 }\n\
     }\n\n\
     fn apply(f: Fn(Int) -> Int, n: Int) -> Int { f(n) }\n\n";

// -- what can cross ---------------------------------------------------------

#[test]
fn a_closure_can_be_an_argument() {
    let src = format!(
        "{LOG}\
         fn f(n: Int) -> Int {{ apply(|x: Int| x + 1, n) }}\n\n\
         test \"it runs\" {{\n\
         \x20 assert f(1) == 2\n\
         }}\n"
    );
    let (sources, checked) = expect_clean(&src);
    expect_tests_pass(&sources, &checked);
}

#[test]
fn a_declared_function_can_be_an_argument() {
    // A function named where a value belongs is a value, and calling it that
    // way has to go through the same path a written out call takes or every
    // contract on it would be skipped by passing it rather than naming it.
    let src = format!(
        "{LOG}\
         fn double(n: Int) -> Int {{ n + n }}\n\n\
         fn f(n: Int) -> Int {{ apply(double, n) }}\n\n\
         test \"it runs\" {{\n\
         \x20 assert f(3) == 6\n\
         }}\n"
    );
    let (sources, checked) = expect_clean(&src);
    expect_tests_pass(&sources, &checked);
}

#[test]
fn a_function_value_can_be_passed_on() {
    let src = format!(
        "{LOG}\
         fn twice(f: Fn(Int) -> Int, n: Int) -> Int {{ apply(f, apply(f, n)) }}\n\n\
         fn double(n: Int) -> Int {{ n + n }}\n\n\
         test \"it runs\" {{\n\
         \x20 assert twice(double, 2) == 8\n\
         }}\n"
    );
    let (sources, checked) = expect_clean(&src);
    expect_tests_pass(&sources, &checked);
}

#[test]
fn a_closure_can_be_returned() {
    // The thing that could not be done at all before this: there was no syntax
    // for the type, so a closure could not leave the function that wrote it.
    let src = format!(
        "{LOG}\
         fn adder() -> Fn(Int) -> Int {{ |x: Int| x + 1 }}\n\n\
         test \"it runs\" {{\n\
         \x20 assert apply(adder(), 1) == 2\n\
         }}\n"
    );
    let (sources, checked) = expect_clean(&src);
    expect_tests_pass(&sources, &checked);
}

#[test]
fn a_local_holding_a_closure_can_be_passed() {
    let src = format!(
        "{LOG}\
         fn f(n: Int) -> Int {{\n\
         \x20 let step = |x: Int| x - 1\n\
         \x20 apply(step, n)\n\
         }}\n\n\
         test \"it runs\" {{\n\
         \x20 assert f(5) == 4\n\
         }}\n"
    );
    let (sources, checked) = expect_clean(&src);
    expect_tests_pass(&sources, &checked);
}

#[test]
fn a_function_value_carries_its_contract() {
    // The reason a function value is not a closure: a closure has no contract
    // and a function does, so passing one by name has to keep the `where`.
    let src = format!(
        "{LOG}\
         fn halved(n: Int) -> Int\n\
         \x20 where\n\
         \x20   n > 0,\n\
         {{\n\
         \x20 n\n\
         }}\n\n\
         test \"the precondition still runs\" {{\n\
         \x20 assert apply(halved, 0) == 0\n\
         }}\n"
    );
    let (sources, checked) = expect_clean(&src);

    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        checked.guards(),
    );
    let mut outcomes = run_tests(&program, checked.file);
    let failure = outcomes
        .remove(0)
        .failure
        .expect("the precondition should have refused zero");
    assert_eq!(
        failure.code,
        vow_interp::codes::PRECONDITION_FAILED,
        "{}",
        render_human(&sources, &failure)
    );
}

// -- what cannot --------------------------------------------------------------

#[test]
fn a_closure_that_performs_an_effect_cannot_cross() {
    let text = expect_refused(&format!(
        "{LOG}\
         fn f(n: Int) -> Int\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {{\n\
         \x20 apply(|x: Int| {{\n\
         \x20   Log.note(\"in here\")\n\
         \x20   x\n\
         \x20 }}, n)\n\
         }}\n"
    ));
    assert!(text.contains("VOW5007"), "{text}");
    assert!(text.contains("a function type promises nothing"), "{text}");
}

#[test]
fn a_function_that_performs_an_effect_cannot_cross() {
    let text = expect_refused(&format!(
        "{LOG}\
         fn noisy(n: Int) -> Int\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {{\n\
         \x20 Log.note(\"hello\")\n\
         \x20 n\n\
         }}\n\n\
         fn f(n: Int) -> Int {{ apply(noisy, n) }}\n"
    ));
    assert!(text.contains("VOW5007"), "{text}");
}

#[test]
fn a_local_holding_an_effectful_closure_cannot_cross() {
    // The interesting one. The closure is not written where the function type
    // is wanted, so the row has to be remembered under the name.
    let text = expect_refused(&format!(
        "{LOG}\
         fn f(n: Int) -> Int\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {{\n\
         \x20 let step = |x: Int| {{\n\
         \x20   Log.note(\"later\")\n\
         \x20   x\n\
         \x20 }}\n\
         \x20 apply(step, n)\n\
         }}\n"
    ));
    assert!(text.contains("VOW5007"), "{text}");
}

#[test]
fn an_effectful_closure_cannot_be_returned() {
    let text = expect_refused(&format!(
        "{LOG}\
         fn escape() -> Fn(Int) -> Int\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {{\n\
         \x20 |x: Int| {{\n\
         \x20   Log.note(\"escaping\")\n\
         \x20   x\n\
         \x20 }}\n\
         }}\n"
    ));
    assert!(text.contains("VOW5007"), "{text}");
}

#[test]
fn a_function_value_of_the_wrong_shape_is_a_type_error() {
    let text = expect_refused(&format!(
        "{LOG}\
         fn two(a: Int, b: Int) -> Int {{ a + b }}\n\n\
         fn f(n: Int) -> Int {{ apply(two, n) }}\n"
    ));
    assert!(text.contains("VOW4"), "{text}");
}

fn expect_tests_pass(sources: &SourceMap, checked: &Checked) {
    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        checked.guards(),
    );
    let outcomes = run_tests(&program, checked.file);
    assert!(!outcomes.is_empty(), "no tests were found");
    for outcome in &outcomes {
        if let Some(failure) = &outcome.failure {
            panic!(
                "`{}` should have passed:\n{}",
                outcome.name,
                render_human(sources, failure)
            );
        }
    }
}
