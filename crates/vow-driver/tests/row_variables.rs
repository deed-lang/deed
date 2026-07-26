//! Row variables.
//!
//! Before these there were two ways to write `map` and both were wrong. One
//! took `Fn(A) -> B`, which promises to perform nothing, so the callback could
//! not log or read a file. The other took `Fn(A) uses Log.note -> B`, which
//! works for exactly one effect and needs a second copy for the next one.
//!
//! `uses r` stands for whatever the callback performs, and `uses r` on the
//! function passes that through to its own row. One `map`, any callback, and
//! a caller that has to declare what its own callback does.
//!
//! Everything here runs the whole pipeline. Which row a variable stands for is
//! decided at the call site by the effect checker, and whether the value
//! crossing into the type was allowed is decided by the type checker, so no
//! single pass sees all of it.

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

/// An effect, a handler for it, and one `map` for every callback there is.
const LIBRARY: &str = "module a\n\n\
     effect Log {\n\
     \x20 fn note(message: String) -> ()\n\
     }\n\n\
     handler Counted implements Log {\n\
     \x20 state seen: Int\n\n\
     \x20 fn note(message) -> () {\n\
     \x20   seen = seen + 1\n\
     \x20 }\n\
     }\n\n\
     fn map<A, B, uses r>(items: List<A>, step: Fn(A) uses r -> B) -> List<B>\n\
     \x20 uses r,\n\
     {\n\
     \x20 for item in items with out = [] {\n\
     \x20   push(out, step(item))\n\
     \x20 }\n\
     }\n\n";

// -- one declaration, every callback ------------------------------------------

#[test]
fn a_callback_that_performs_nothing_costs_the_caller_nothing() {
    // The row variable came back empty, so the signature says nothing and
    // that is not an omission.
    expect_clean(&format!(
        "{LIBRARY}\
         fn doubled(ns: List<Int>) -> List<Int> {{ map(ns, |n: Int| n + n) }}\n"
    ));
}

#[test]
fn a_callback_that_performs_an_effect_costs_the_caller_that_effect() {
    expect_clean(&format!(
        "{LIBRARY}\
         fn announced(ns: List<Int>) -> List<Int>\n\
         \x20 uses Log.note,\n\
         {{\n\
         \x20 map(ns, |n: Int| {{\n\
         \x20   Log.note(\"saw one\")\n\
         \x20   n + 1\n\
         \x20 }})\n\
         }}\n"
    ));
}

#[test]
fn one_map_serves_both_and_the_two_callers_declare_differently() {
    // The whole argument for a row variable in one file. Before this there
    // would be two `map`s, and a third for the next effect.
    expect_tests_pass(&format!(
        "{LIBRARY}\
         fn doubled(ns: List<Int>) -> List<Int> {{ map(ns, |n: Int| n + n) }}\n\n\
         fn announced(ns: List<Int>) -> List<Int>\n\
         \x20 uses Log.note,\n\
         {{\n\
         \x20 map(ns, |n: Int| {{\n\
         \x20   Log.note(\"saw one\")\n\
         \x20   n + 1\n\
         \x20 }})\n\
         }}\n\n\
         test \"both\" {{\n\
         \x20 assert doubled([1, 2]) == [2, 4]\n\
         \x20 with Counted {{ seen: 0 }} {{\n\
         \x20   assert announced([1, 2]) == [2, 3]\n\
         \x20 }}\n\
         }}\n"
    ));
}

#[test]
fn a_named_function_passes_its_declared_row_through() {
    // A declared function named where a value belongs carries its contract,
    // and that is what flows into the row variable.
    expect_clean(&format!(
        "{LIBRARY}\
         fn shout(n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {{\n\
         \x20 Log.note(\"shouting\")\n\
         \x20 n\n\
         }}\n\n\
         fn all_shouted(ns: List<Int>) -> List<Int>\n\
         \x20 uses Log.note,\n\
         {{ map(ns, shout) }}\n"
    ));
}

// -- what it still refuses -------------------------------------------------------

#[test]
fn a_caller_that_does_not_declare_what_it_handed_over_is_refused() {
    // The rule that matters. A row variable makes the library flexible and
    // does not make the caller's row optional.
    let text = expect_refused(&format!(
        "{LIBRARY}\
         fn forgot(ns: List<Int>) -> List<Int> {{\n\
         \x20 map(ns, |n: Int| {{\n\
         \x20   Log.note(\"saw one\")\n\
         \x20   n\n\
         \x20 }})\n\
         }}\n"
    ));
    assert!(text.contains("VOW5001"), "{text}");
    assert!(text.contains("`Log.note`"), "{text}");
}

#[test]
fn a_caller_that_declares_more_than_it_handed_over_is_refused_too() {
    // The other half, and the half that matters more: a row nobody can trust
    // is a row nobody reads.
    let text = expect_refused(&format!(
        "{LIBRARY}\
         fn too_wide(ns: List<Int>) -> List<Int>\n\
         \x20 uses Log.note,\n\
         {{ map(ns, |n: Int| n + n) }}\n"
    ));
    assert!(text.contains("VOW5002"), "{text}");
}

#[test]
fn a_row_variable_that_reaches_no_parameter_is_refused() {
    // Caught by the rule that was already there. A declared row has to be
    // performed, and a variable nothing can fill is one nothing performs, so
    // the existing "too wide" check says so with no new code behind it.
    let text = expect_refused(
        "module a\n\n\
         fn silent<A, B, uses r>(items: List<A>, step: Fn(A) -> B) -> List<B>\n\
         \x20 uses r,\n\
         {\n\
         \x20 for item in items with out = [] {\n\
         \x20   push(out, step(item))\n\
         \x20 }\n\
         }\n",
    );
    assert!(text.contains("VOW5002"), "{text}");
    assert!(
        text.contains("`r` is declared but never performed"),
        "{text}"
    );
}

#[test]
fn a_function_that_calls_its_callback_without_passing_the_row_on_is_refused() {
    // `step` may perform `r`, and this declares nothing, so calling it is the
    // ordinary too-narrow error.
    let text = expect_refused(
        "module a\n\n\
         fn map<A, B, uses r>(items: List<A>, step: Fn(A) uses r -> B) -> List<B> {\n\
         \x20 for item in items with out = [] {\n\
         \x20   push(out, step(item))\n\
         \x20 }\n\
         }\n",
    );
    assert!(text.contains("VOW5001"), "{text}");
}

// -- where a row variable may be written ---------------------------------------
//
// It stands for whatever a callback performs, and the only thing that knows
// what that was is the call that supplied the callback. So it has to be
// readable off an argument. Written anywhere else it arrives at a caller
// naming something the caller has no word for, which means it gets dropped,
// and a dropped entry is an effect that happens and is not declared.

#[test]
fn a_row_variable_in_a_return_type_is_refused() {
    let text = expect_refused(
        "module a\n\n\
         fn pick<A, uses r>(f: Fn(A) uses r -> A, left: Bool) -> Fn(A) uses r -> A\n\
         \x20 uses r,\n\
         { f }\n",
    );
    assert!(text.contains("VOW5008"), "{text}");
    assert!(text.contains("`r` is a row variable"), "{text}");
}

#[test]
fn a_row_variable_buried_inside_a_parameter_is_refused_too() {
    // The same hole from the other side. Only a parameter that is itself a
    // function type is looked at when a call works out what a variable stood
    // for, so one hiding inside a `List` would never be filled in.
    let text = expect_refused(
        "module a\n\n\
         fn run<A, uses r>(steps: List<Fn(A) uses r -> A>, start: A) -> A\n\
         \x20 uses r,\n\
         {\n\
         \x20 for step in steps with value = start { step(value) }\n\
         }\n",
    );
    assert!(text.contains("VOW5008"), "{text}");
}

#[test]
fn the_one_legal_position_is_still_legal() {
    // The rule has to leave the shape the whole feature exists for alone.
    expect_clean(
        "module a\n\n\
         fn map<A, B, uses r>(items: List<A>, step: Fn(A) uses r -> B) -> List<B>\n\
         \x20 uses r,\n\
         {\n\
         \x20 for item in items with out = [] {\n\
         \x20   push(out, step(item))\n\
         \x20 }\n\
         }\n",
    );
}

// -- across a module boundary -----------------------------------------------------

/// A library module holding nothing but a `map` with a row variable.
const LIST: &str = "module list\n\n\
     fn map<A, B, uses r>(items: List<A>, step: Fn(A) uses r -> B) -> List<B>\n\
     \x20 uses r,\n\
     {\n\
     \x20 for item in items with out = [] {\n\
     \x20   push(out, step(item))\n\
     \x20 }\n\
     }\n";

fn check_with_library(app: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = [app, LIST]
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("file{index}.vow"), *text))
        .collect();
    let mut checks = check_all(&sources, &ids);
    (sources, checks.remove(0))
}

#[test]
fn a_row_variable_survives_a_module_boundary() {
    // A position rather than a name crosses, the same way a type parameter
    // does, because a `DefId` means nothing outside the table it came from.
    let (sources, checked) = check_with_library(
        "module app\n\n\
         use list.{map}\n\n\
         effect Log {\n\
         \x20 fn note(message: String) -> ()\n\
         }\n\n\
         fn quiet(ns: List<Int>) -> List<Int> { map(ns, |n: Int| n + n) }\n\n\
         fn loud(ns: List<Int>) -> List<Int>\n\
         \x20 uses Log.note,\n\
         {\n\
         \x20 map(ns, |n: Int| {\n\
         \x20   Log.note(\"saw one\")\n\
         \x20   n\n\
         \x20 })\n\
         }\n",
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn an_imported_map_still_charges_the_caller() {
    let (sources, checked) = check_with_library(
        "module app\n\n\
         use list.{map}\n\n\
         effect Log {\n\
         \x20 fn note(message: String) -> ()\n\
         }\n\n\
         fn forgot(ns: List<Int>) -> List<Int> {\n\
         \x20 map(ns, |n: Int| {\n\
         \x20   Log.note(\"saw one\")\n\
         \x20   n\n\
         \x20 })\n\
         }\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(checked.has_errors(), "this should not have been accepted");
    assert!(text.contains("VOW5001"), "{text}");
    assert!(text.contains("`Log.note`"), "{text}");
}
