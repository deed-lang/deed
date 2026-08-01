//! Assignment, which is only allowed to handler state.
//!
//! The restriction is the point, so most of these are about what is rejected.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::check_text;

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

const COUNTER: &str = "\
module a

effect Counter {
    fn value() -> Int
    fn bump(by: Int) -> ()
}
";

#[test]
fn a_handler_may_change_its_own_state() {
    check_ok(&format!(
        "{COUNTER}\n\
         handler InMemory implements Counter {{\n\
         \x20   state count: Int\n\n\
         \x20   fn value() -> Int {{ count }}\n\
         \x20   fn bump(by) -> () {{ count = count + by }}\n\
         }}\n"
    ));
}

#[test]
fn assigning_to_a_parameter_is_rejected() {
    let (sources, checked) = check("module a\n\nfn f(n: Int) -> Int {\n  n = 1\n  n\n}\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::NOT_ASSIGNABLE]
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("`n` is a parameter, not handler state"),
        "{text}"
    );
    assert!(text.contains("only mutable thing in Deed"), "{text}");
}

#[test]
fn assigning_to_a_let_binding_is_rejected() {
    let (sources, checked) = check("module a\n\nfn f() -> Int {\n  let x = 1\n  x = 2\n  x\n}\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::NOT_ASSIGNABLE]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("is a binding"));
}

/// The shape somebody reaching for a mutable name inside a walk actually
/// wanted, written with their own names.
///
/// A model given the total task wrote `let sum = 0` and then `sum = sum + n`
/// inside a `for`, which is how almost every other language spells it, and
/// spent five checks in a row being told the rule. The rule was right and it
/// does not say what to write.
#[test]
fn assigning_inside_a_walk_is_shown_the_accumulator() {
    let (sources, checked) = check(
        "module a\n\nfn total(numbers: List<Int>) -> Int {\n  let sum = 0\n  \
         for number in numbers {\n    sum = sum + number\n  }\n  sum\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("for number in ... with sum = ... { ... }"),
        "{text}"
    );
}

/// And not outside one, where it would be pointing at a loop that is not there.
#[test]
fn assigning_outside_a_walk_is_not_shown_an_accumulator() {
    let (sources, checked) = check("module a\n\nfn f() -> Int {\n  let x = 1\n  x = 2\n  x\n}\n");
    assert!(
        !rendered(&sources, &checked.diagnostics).contains("carries what it is building"),
        "there is no walk here to carry anything"
    );
}

/// The innermost one, since that is the walk the assignment is in.
#[test]
fn a_nested_walk_names_the_binder_that_is_actually_around_it() {
    let (sources, checked) = check(
        "module a\n\nfn f(rows: List<List<Int>>) -> Int {\n  let total = 0\n  \
         for row in rows {\n    for cell in row {\n      total = total + cell\n    }\n  }\n  \
         total\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("for cell in ..."), "{text}");
}

#[test]
fn the_value_is_checked_against_the_declared_state_type() {
    let (sources, checked) = check(&format!(
        "{COUNTER}\n\
         handler InMemory implements Counter {{\n\
         \x20   state count: Int\n\n\
         \x20   fn value() -> Int {{ count }}\n\
         \x20   fn bump(by) -> () {{ count = true }}\n\
         }}\n"
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::TYPE_MISMATCH]
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("expected `Int`, found `Bool`"), "{text}");
    assert!(text.contains("the state it is assigned to"), "{text}");
}

#[test]
fn an_assignment_carries_the_effects_of_its_right_hand_side() {
    let (sources, checked) = check(&format!(
        "{COUNTER}\n\
         effect Ticker {{\n\
         \x20   fn now() -> Int\n\
         }}\n\n\
         handler InMemory implements Counter {{\n\
         \x20   state count: Int\n\n\
         \x20   fn value() -> Int {{ count }}\n\
         \x20   fn bump(by) -> () {{ count = Ticker.now() }}\n\
         }}\n"
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_effects::codes::UNDECLARED_EFFECT],
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn an_assignment_to_an_unknown_name_says_so_once() {
    let (_, checked) = check("module a\n\nfn f() -> Int {\n  nope = 1\n  0\n}\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_resolve::codes::UNKNOWN_NAME]
    );
}

#[test]
fn equality_is_still_an_expression() {
    // `x == 1` must not be mistaken for an assignment.
    check_ok("module a\n\nfn f(x: Int) -> Bool {\n  x == 1\n}\n");
}

#[test]
fn a_statement_after_an_assignment_still_parses() {
    check_ok(&format!(
        "{COUNTER}\n\
         handler InMemory implements Counter {{\n\
         \x20   state count: Int\n\n\
         \x20   fn value() -> Int {{ count }}\n\
         \x20   fn bump(by) -> () {{\n\
         \x20       count = count + by\n\
         \x20       count = count + 1\n\
         \x20   }}\n\
         }}\n"
    ));
}
