//! Generic functions.
//!
//! `Result` and `List` are built in because there was no way to declare one,
//! and the real cost of that was not the two shortcuts. It was that nobody
//! could write a library: `first`, `map`, `filter` and `any` are all the same
//! function at different element types, and none of them could be written
//! down.
//!
//! There is no unification here and no global inference, and generics do not
//! bring either. At a call site the declared parameter types are matched
//! against the argument types, walking down both in step, and that is the
//! whole mechanism.
//!
//! Every type parameter has to appear in a parameter's type, which is what
//! removes explicit type arguments and with them the `f<a>(b)` versus
//! `f < a > (b)` ambiguity.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::{Checked, check_all, check_text};
use deed_interp::{Program, run_tests};

fn check(src: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.deed", src.to_string());
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

/// Runs a file's tests, which are expected to pass.
fn expect_tests_pass(src: &str) {
    let (sources, checked) = expect_clean(src);
    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        checked.guards(),
        checked.rows(),
        checked.operators(),
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

const FIRST: &str = "module a\n\n\
     fn first<T>(items: List<T>) -> Result<T, String> {\n\
     \x20   at(items, 0)\n\
     }\n\n";

// -- what a call works out ----------------------------------------------------

#[test]
fn a_type_parameter_comes_from_the_argument() {
    expect_clean(&format!(
        "{FIRST}\
         fn head(names: List<String>) -> Result<String, String> {{ first(names) }}\n\n\
         fn count(ns: List<Int>) -> Result<Int, String> {{ first(ns) }}\n"
    ));
}

#[test]
fn one_declaration_serves_every_element_type() {
    expect_tests_pass(&format!(
        "{FIRST}\
         test \"it works at three types\" {{\n\
         \x20 assert first([1, 2, 3]) == ok(1)\n\
         \x20 assert first([\"a\"]) == ok(\"a\")\n\
         \x20 assert first([true]) == ok(true)\n\
         }}\n"
    ));
}

#[test]
fn the_answer_is_the_argument_type_and_not_something_wider() {
    // The point of working it out rather than leaving it unknown. An unknown
    // agrees with everything, so a `first` that handed one back would make
    // every use of its result unchecked.
    //
    // The complaint lands on the payload rather than on the `Result` around
    // it, which is what `Result` being compared componentwise buys.
    let text = expect_refused(&format!(
        "{FIRST}\
         fn wrong(names: List<String>) -> Result<Int, String> {{ first(names) }}\n"
    ));
    assert!(text.contains("DEED4001"), "{text}");
    assert!(text.contains("expected `Int`, found `String`"), "{text}");
}

#[test]
fn a_parameter_nested_two_deep_is_still_found() {
    expect_clean(
        "module a\n\n\
         fn unwrap_all<T>(items: List<Result<T, String>>) -> Int { length(items) }\n\n\
         fn use_it(rows: List<Result<Bool, String>>) -> Int { unwrap_all(rows) }\n",
    );
}

#[test]
fn a_function_type_carries_one_through() {
    expect_tests_pass(
        "module a\n\n\
         fn apply<A, B>(f: Fn(A) -> B, value: A) -> B { f(value) }\n\n\
         test \"it works\" {\n\
         \x20 assert apply(|n: Int| n + 1, 1) == 2\n\
         \x20 assert apply(|s: String| length(s), \"abc\") == 3\n\
         }\n",
    );
}

#[test]
fn calling_a_function_typed_parameter_stays_at_the_declarations_own_types() {
    // The `A` and `B` in a `f: Fn(A) -> B` parameter belong to the function
    // that declared `f`, not to `f`. Inside that body they are settled, so
    // `f(value)` is a `B` and there is nothing to work out.
    //
    // Getting this wrong made `step(item)` in a `map` have no type at all,
    // which `fully_typed.rs` caught, which is what that invariant is for.
    expect_clean(
        "module a\n\n\
         fn map<A, B>(items: List<A>, step: Fn(A) -> B) -> List<B> {\n\
         \x20 for item in items with out = [] {\n\
         \x20   push(out, step(item))\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn the_first_argument_decides_and_the_second_is_checked_against_it() {
    // Rather than a unifier complaining about a variable nobody wrote. The
    // message names the type the call already settled on and points at the
    // parameter that disagreed with it.
    let text = expect_refused(
        "module a\n\n\
         fn same<T>(a: T, b: T) -> Int { 0 }\n\n\
         fn f() -> Int { same(1, \"two\") }\n",
    );
    assert!(text.contains("DEED4001"), "{text}");
    assert!(text.contains("expected `Int`, found `String`"), "{text}");
}

#[test]
fn an_argument_the_checker_gave_up_on_decides_nothing() {
    // An unknown agrees with whatever the parameter turns out to be, so
    // treating it as an answer would let one argument nobody could type decide
    // the type of every other.
    let text = expect_refused(
        "module a\n\n\
         fn same<T>(a: T, b: T) -> Int { 0 }\n\n\
         fn f() -> Int { same(nothing, 1) }\n",
    );
    assert!(text.contains("DEED3001"), "{text}");
    // One complaint, about the name that does not exist. Nothing about `b`.
    assert!(!text.contains("DEED4001"), "{text}");
}

// -- what a declaration has to say --------------------------------------------

#[test]
fn a_type_parameter_that_appears_in_no_parameter_is_refused() {
    let text = expect_refused("module a\n\nfn empty<T>() -> List<T> { [] }\n");
    assert!(text.contains("DEED4023"), "{text}");
    assert!(
        text.contains("nothing at a call site says what `T` is"),
        "{text}"
    );
    // The note says why the return type does not count, which is the thing
    // somebody writing this would want to know.
    assert!(
        text.contains("a return type is what a call produces"),
        "{text}"
    );
}

#[test]
fn a_type_parameter_used_nowhere_at_all_is_refused_too() {
    let text = expect_refused("module a\n\nfn odd<T>(n: Int) -> Int { n }\n");
    assert!(text.contains("DEED4023"), "{text}");
    assert!(text.contains("nothing to match"), "{text}");
}

#[test]
fn a_generic_function_is_not_a_value() {
    // One expression has one type here, and a generic function named rather
    // than called has as many as there are ways to call it.
    let text = expect_refused(&format!(
        "{FIRST}\
         fn apply(f: Fn(List<Int>) -> Result<Int, String>) -> Int {{ 0 }}\n\n\
         fn f() -> Int {{ apply(first) }}\n"
    ));
    assert!(text.contains("DEED4024"), "{text}");
    assert!(text.contains("nothing here says what `T` is"), "{text}");
}

#[test]
fn a_type_parameter_is_not_a_free_pass_inside_the_body() {
    // `T` is whatever the caller decides, so the body knows nothing about it
    // and may not assume it is an `Int`.
    let text = expect_refused("module a\n\nfn wrong<T>(value: T) -> Int { value }\n");
    assert!(text.contains("DEED4001"), "{text}");
    assert!(text.contains("`T`"), "{text}");
}

#[test]
fn a_generic_builtin_is_not_a_value_either() {
    // The same rule as the test above, on the prelude's own generic names.
    // They have no signature to hand back, and the unknown that stood in for
    // one absorbed, so each of these compared equal to an `Int` and reached an
    // interpreter with no value to give it. Named one at a time so a missing
    // one says which.
    for name in ["ok", "err", "at", "push", "repeat"] {
        let text = expect_refused(&format!(
            "module a\n\nfn f(n: Int) -> Bool {{ {name} == n }}\n"
        ));
        assert!(text.contains("DEED4019"), "{name}: {text}");
        // The message, not the note. The note is added from the same list by a
        // separate test, so asking only for it let the refusal fall through to
        // the arm that calls these a type and still pass.
        assert!(
            text.contains(&format!("`{name}` is a builtin that works on any type")),
            "{name}: {text}"
        );
    }
}

#[test]
fn a_generic_builtin_is_not_a_value_in_a_contract_clause() {
    // #818, and the whole of why this one was missed: a model reached for `at`
    // as a way to say `result`, the check said nothing nine times over, and the
    // interpreter refused it with a note saying the check had a hole.
    let text = expect_refused(
        "module a\n\n\
         fn f(n: Int) -> Int\n    ensures ok => at == n\n{\n    n\n}\n",
    );
    assert!(text.contains("DEED4019"), "{text}");
    assert!(
        text.contains("`at` is a builtin that works on any type"),
        "{text}"
    );
    assert!(text.contains("call it rather than naming it"), "{text}");
}

#[test]
fn a_type_parameter_shadowing_a_type_is_said_out_loud() {
    let (sources, checked) = check("module a\n\nfn f<Int>(value: Int) -> Int { value }\n");
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("DEED3005"), "{text}");
    assert!(text.contains("hides"), "{text}");
}

// -- across a module boundary -------------------------------------------------

#[test]
fn a_generic_function_can_be_imported_and_still_works_out() {
    // A type parameter crosses as a position rather than as a `DefId`, for the
    // same reason an imported type crosses as a module path and a name.
    let mut sources = SourceMap::new();
    let ids: Vec<_> = [
        "module app\n\n\
         use lib.{first}\n\n\
         fn head(names: List<String>) -> String {\n\
         \x20 match first(names) {\n\
         \x20   ok(name) => name,\n\
         \x20   err(why) => \"none\",\n\
         \x20 }\n\
         }\n",
        "module lib\n\n\
         fn first<T>(items: List<T>) -> Result<T, String> { at(items, 0) }\n",
    ]
    .iter()
    .enumerate()
    .map(|(index, text)| sources.add(format!("file{index}.deed"), *text))
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
fn an_imported_generic_function_is_checked_at_the_type_it_was_called_with() {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = [
        "module app\n\n\
         use lib.{first}\n\n\
         fn wrong(names: List<String>) -> Result<Int, String> { first(names) }\n",
        "module lib\n\n\
         fn first<T>(items: List<T>) -> Result<T, String> { at(items, 0) }\n",
    ]
    .iter()
    .enumerate()
    .map(|(index, text)| sources.add(format!("file{index}.deed"), *text))
    .collect();

    let mut checks = check_all(&sources, &ids);
    let checked = checks.remove(0);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(checked.has_errors(), "this should not have been accepted");
    assert!(text.contains("expected `Int`, found `String`"), "{text}");
}
