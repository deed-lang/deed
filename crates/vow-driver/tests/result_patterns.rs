//! Matching on a `Result`.

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::check_text;

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

const PRELUDE: &str = "\
choice Failure {
    TooBig { limit: Int },
    Empty,
}

fn small(n: Int) -> Result<Int, Failure> {
    if n > 10 {
        return err(TooBig { limit: 10 })
    }
    ok(n)
}
";

fn with_prelude(rest: &str) -> String {
    format!("module a\n\n{PRELUDE}\n{rest}")
}

// -- binding ---------------------------------------------------------------

#[test]
fn both_cases_can_be_bound_with_the_right_types() {
    check_ok(&with_prelude(
        "fn handled(n: Int) -> Int {\n\
         \x20 match small(n) {\n\
         \x20   ok(value) => value + 1,\n\
         \x20   err(TooBig { limit }) => limit,\n\
         \x20   err(Empty) => 0,\n\
         \x20 }\n\
         }\n",
    ));
}

#[test]
fn the_success_binding_has_the_success_type() {
    let (sources, checked) = check(&with_prelude(
        "fn handled(n: Int) -> Int {\n\
         \x20 match small(n) {\n\
         \x20   ok(value) => !value,\n\
         \x20   err(other) => 0,\n\
         \x20 }\n\
         }\n",
    ));
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::TYPE_MISMATCH),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("expected `Bool`, found `Int`"));
}

#[test]
fn the_failure_binding_has_the_error_type() {
    check_ok(&with_prelude(
        "fn describe(f: Failure) -> Int { 0 }\n\n\
         fn handled(n: Int) -> Int {\n\
         \x20 match small(n) {\n\
         \x20   ok(value) => value,\n\
         \x20   err(problem) => describe(problem),\n\
         \x20 }\n\
         }\n",
    ));
}

// -- exhaustiveness --------------------------------------------------------

#[test]
fn forgetting_the_failure_case_is_an_error() {
    let (sources, checked) = check(&with_prelude(
        "fn handled(n: Int) -> Int {\n\
         \x20 match small(n) {\n\
         \x20   ok(value) => value,\n\
         \x20 }\n\
         }\n",
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::NON_EXHAUSTIVE_MATCH]
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("does not cover `err`"), "{text}");
    assert!(text.contains("both need an arm"), "{text}");
}

#[test]
fn forgetting_the_success_case_is_an_error_too() {
    let (sources, checked) = check(&with_prelude(
        "fn handled(n: Int) -> Int {\n\
         \x20 match small(n) {\n\
         \x20   err(problem) => 0,\n\
         \x20 }\n\
         }\n",
    ));
    assert!(rendered(&sources, &checked.diagnostics).contains("does not cover `ok`"));
}

#[test]
fn a_wildcard_cannot_swallow_the_failure_case() {
    let (sources, checked) = check(&with_prelude(
        "fn handled(n: Int) -> Int {\n\
         \x20 match small(n) {\n\
         \x20   ok(value) => value,\n\
         \x20   _ => 0,\n\
         \x20 }\n\
         }\n",
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::CATCH_ALL_ON_CHOICE]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("cannot be handled by accident"));
}

#[test]
fn a_bare_binding_cannot_swallow_it_either() {
    let (_, checked) = check(&with_prelude(
        "fn handled(n: Int) -> Int {\n\
         \x20 match small(n) {\n\
         \x20   ok(value) => value,\n\
         \x20   anything => 0,\n\
         \x20 }\n\
         }\n",
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![vow_typeck::codes::CATCH_ALL_ON_CHOICE]
    );
}

// -- patterns that cannot match --------------------------------------------

#[test]
fn ok_on_something_that_is_not_a_result_is_rejected() {
    let (sources, checked) = check(
        "module a\n\nfn f(n: Int) -> Int {\n  match n {\n    ok(x) => x,\n    err(e) => 0,\n  }\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::PATTERN_MISMATCH),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("`ok(...)` matches a `Result`"));
}

#[test]
fn a_variant_cannot_be_matched_positionally() {
    // Variants have named fields, so `TooBig(x)` can never match anything and
    // should say so rather than silently falling through.
    let (sources, checked) = check(&with_prelude(
        "fn handled(f: Failure) -> Int {\n\
         \x20 match f {\n\
         \x20   TooBig(limit) => limit,\n\
         \x20   Empty => 0,\n\
         \x20 }\n\
         }\n",
    ));
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::PATTERN_MISMATCH),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("only `ok` and `err` carry a value"), "{text}");
    assert!(text.contains("`Variant { field }`"), "{text}");
}

#[test]
fn a_pattern_that_binds_the_wrong_number_of_values_is_rejected() {
    let (sources, checked) = check(&with_prelude(
        "fn handled(n: Int) -> Int {\n\
         \x20 match small(n) {\n\
         \x20   ok(a, b) => a,\n\
         \x20   err(e) => 0,\n\
         \x20 }\n\
         }\n",
    ));
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::PATTERN_MISMATCH),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("binds 2 values"));
}

// -- shadowing still applies -----------------------------------------------

#[test]
fn a_binding_in_a_pattern_cannot_shadow_a_parameter() {
    // Found while writing examples/counter.vow, where the obvious name for the
    // binding was already a parameter.
    let (sources, checked) = check(&with_prelude(
        "fn handled(limit: Int) -> Int {\n\
         \x20 match small(limit) {\n\
         \x20   ok(value) => value,\n\
         \x20   err(TooBig { limit }) => limit,\n\
         \x20   err(Empty) => 0,\n\
         \x20 }\n\
         }\n",
    ));
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_resolve::codes::SHADOWED_BINDING),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn renaming_the_binding_fixes_it() {
    check_ok(&with_prelude(
        "fn handled(limit: Int) -> Int {\n\
         \x20 match small(limit) {\n\
         \x20   ok(value) => value,\n\
         \x20   err(TooBig { limit: reached }) => reached,\n\
         \x20   err(Empty) => 0,\n\
         \x20 }\n\
         }\n",
    ));
}
