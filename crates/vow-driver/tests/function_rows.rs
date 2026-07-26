//! Rows on function types.
//!
//! `Fn(Int) -> Int` says the function performs nothing.
//! `Fn(Int) uses Log.note -> Int` says it performs that and no more. Before
//! this existed, a function type could only say "nothing", which meant a
//! callback that logged could not be written at all and every higher order
//! function in the language was restricted to arithmetic.
//!
//! The row goes before the arrow. After the return type it would be
//! indistinguishable from a declaration's own contract, because that also
//! starts with `uses` and also follows a return type.
//!
//! These run the whole pipeline, and they have to. Which values owe a row is a
//! question about types and whether one is kept is a question about rows, so
//! no single pass can answer it.

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::{Checked, check_all, check_text};

fn check(src: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.vow", src.to_string());
    (sources, checked)
}

/// Checks several files together, and returns the result for the first.
fn check_together(files: &[&str]) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("file{index}.vow"), *text))
        .collect();

    let mut checks = check_all(&sources, &ids);
    (sources, checks.remove(0))
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

fn expect_clean(src: &str) {
    let (sources, checked) = check(src);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
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
     effect Log {\n\
     \x20 fn note(message: String) -> ()\n\
     \x20 fn warn(message: String) -> ()\n\
     }\n\n";

// -- what a row lets through --------------------------------------------------

#[test]
fn a_closure_may_perform_what_the_type_allows() {
    expect_clean(&format!(
        "{LOG}\
         fn apply(f: Fn(Int) uses Log.note -> Int, n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {{ f(n) }}\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {{\n\
         \x20 apply(|x: Int| {{\n\
         \x20   Log.note(\"in here\")\n\
         \x20   x + 1\n\
         \x20 }}, n)\n\
         }}\n"
    ));
}

#[test]
fn a_whole_effect_in_a_row_covers_its_operations() {
    // The same rule a contract follows. `uses Log` in a type has to mean what
    // `uses Log` in a `uses` clause means, or the two spellings of one row
    // would be two languages.
    expect_clean(&format!(
        "{LOG}\
         fn apply(f: Fn(Int) uses Log -> Int, n: Int) -> Int\n\
         \x20 uses Log,\n\
         {{ f(n) }}\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses Log,\n\
         {{\n\
         \x20 apply(|x: Int| {{\n\
         \x20   Log.warn(\"in here\")\n\
         \x20   x\n\
         \x20 }}, n)\n\
         }}\n"
    ));
}

#[test]
fn performing_less_than_the_row_allows_is_fine() {
    // The one place in the checker where a type fits another without being it.
    // It gives way in the safe direction: a function that performs less than
    // it was given room for breaks nothing.
    expect_clean(&format!(
        "{LOG}\
         fn apply(f: Fn(Int) uses Log.note -> Int, n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {{ f(n) }}\n\n\
         fn quiet(x: Int) -> Int {{ x + 1 }}\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {{ apply(quiet, n) }}\n"
    ));
}

// -- what it keeps out --------------------------------------------------------

#[test]
fn a_closure_may_not_perform_more_than_the_type_allows() {
    let text = expect_refused(&format!(
        "{LOG}\
         fn apply(f: Fn(Int) uses Log.note -> Int, n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {{ f(n) }}\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses Log,\n\
         {{\n\
         \x20 apply(|x: Int| {{\n\
         \x20   Log.warn(\"in here\")\n\
         \x20   x\n\
         \x20 }}, n)\n\
         }}\n"
    ));
    assert!(text.contains("VOW5007"), "{text}");
    assert!(text.contains("`Log.warn`"), "{text}");
    // The message says what there was room for, because the useful next thing
    // to know is what the type would have to say instead.
    assert!(text.contains("room only for `Log.note`"), "{text}");
}

#[test]
fn a_declared_function_may_not_perform_more_than_the_type_allows() {
    let text = expect_refused(&format!(
        "{LOG}\
         fn apply(f: Fn(Int) uses Log.note -> Int, n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {{ f(n) }}\n\n\
         fn loud(x: Int) -> Int\n\
         \x20 uses Log.warn,\n\
         {{\n\
         \x20 Log.warn(\"loud\")\n\
         \x20 x\n\
         }}\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses Log,\n\
         {{ apply(loud, n) }}\n"
    ));
    assert!(text.contains("VOW5007"), "{text}");
    assert!(text.contains("`Log.warn`"), "{text}");
}

#[test]
fn a_row_in_a_type_names_effects() {
    let text = expect_refused(
        "module a\n\n\
         fn apply(f: Fn(Int) uses Int -> Int, n: Int) -> Int { f(n) }\n",
    );
    assert!(text.contains("VOW5003"), "{text}");
}

// -- what calling one costs ---------------------------------------------------

#[test]
fn calling_a_function_value_charges_its_row_to_the_caller() {
    // Without this the row would be checked where the value was handed over
    // and then forgotten, so a function taking a callback that logs could
    // declare nothing itself and the row would stop at the parameter.
    let text = expect_refused(&format!(
        "{LOG}\
         fn apply(f: Fn(Int) uses Log.note -> Int, n: Int) -> Int {{ f(n) }}\n"
    ));
    assert!(text.contains("VOW5001"), "{text}");
    assert!(text.contains("`Log.note`"), "{text}");
}

#[test]
fn a_row_the_callback_cannot_reach_is_still_too_wide() {
    // The other half of the rule, and the half that matters more: a row
    // nobody can trust is a row nobody reads.
    let text = expect_refused(&format!(
        "{LOG}\
         fn apply(f: Fn(Int) -> Int, n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {{ f(n) }}\n"
    ));
    assert!(text.contains("VOW5002"), "{text}");
}

// -- how it reads -------------------------------------------------------------

#[test]
fn a_row_is_part_of_the_type_in_a_message() {
    let text = expect_refused(&format!(
        "{LOG}\
         fn apply(f: Fn(Int) uses Log.note -> Int, n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {{ f(n) }}\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {{ apply(1, n) }}\n"
    ));
    assert!(
        text.contains("Fn(Int) uses Log.note -> Int"),
        "the row should be written where the type is: {text}"
    );
}

// -- across a module boundary -------------------------------------------------

/// A module declaring `Log` and a function whose callback may use it.
const HOST: &str = "module other\n\n\
     effect Log {\n\
     \x20 fn note(message: String) -> ()\n\
     \x20 fn warn(message: String) -> ()\n\
     }\n\n\
     fn apply(f: Fn(Int) uses Log.note -> Int, n: Int) -> Int\n\
     \x20 uses Log.note,\n\
     { f(n) }\n";

#[test]
fn a_row_survives_a_module_boundary() {
    let (sources, checked) = check_together(&[
        "module a\n\n\
         use other.{Log, apply}\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {\n\
         \x20 apply(|x: Int| {\n\
         \x20   Log.note(\"in here\")\n\
         \x20   x + 1\n\
         \x20 }, n)\n\
         }\n",
        HOST,
    ]);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn an_imported_row_is_still_a_limit() {
    // The row entry names the module its effect was declared in, so the two
    // sides agree on which `Log` is meant without sharing any numbering.
    let (sources, checked) = check_together(&[
        "module a\n\n\
         use other.{Log, apply}\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses Log,\n\
         {\n\
         \x20 apply(|x: Int| {\n\
         \x20   Log.warn(\"in here\")\n\
         \x20   x\n\
         \x20 }, n)\n\
         }\n",
        HOST,
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(checked.has_errors(), "this should not have been accepted");
    assert!(text.contains("VOW5007"), "{text}");
    assert!(text.contains("`Log.warn`"), "{text}");
}
