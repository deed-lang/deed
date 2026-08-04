//! What the rule in `deed_mir::only_pushes` refuses, asked one shape at a time.
//!
//! `crates/deed-driver/tests/walks.rs` counts how much of the library the rule
//! accepts, which is the measurement the change rests on, but a corpus only
//! contains the shapes somebody happened to write. The dangerous direction is
//! the other one: a body that keeps the accumulator somewhere and is accepted
//! anyway compiles without complaint and answers differently, so every place
//! a name can be kept needs a case here whether or not the library has one.
//!
//! See `design/decisions/2026-08-04-a-walk-that-only-pushes.md`.

use deed_ast::{Accumulator, Expr, Item};
use deed_mir::only_pushes;

/// Whether the rule accepts a walk whose body is this.
fn accepts(body: &str) -> bool {
    let source = format!(
        "module a\n\nfn f(items: List<Int>) -> List<Int> {{\n    for item in items with out = [] {{\n{body}\n    }}\n}}\n"
    );

    let mut sources = deed_diagnostics::SourceMap::new();
    let file = sources.add("shape.deed".to_string(), source.clone());
    let lexed = deed_lexer::tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "this body should lex:\n{source}");
    let parsed = deed_parser::parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "this body should parse:\n{source}");

    let Some(Item::Function(decl)) = parsed.module.items.first() else {
        panic!("the module holds one function");
    };
    let Some(Expr::For {
        accumulator: Some(Accumulator { name, .. }),
        body,
        ..
    }) = decl.body.tail.as_deref()
    else {
        panic!("the function's value should be a walk carrying an accumulator");
    };

    only_pushes(&name.name, body)
}

/// The shape the rule is for.
#[test]
fn a_walk_that_only_pushes_is_accepted() {
    assert!(accepts("        push(out, item)"));
}

/// A branch handing the accumulator on untouched, which is `filter`.
#[test]
fn a_branch_that_hands_the_accumulator_on_is_accepted() {
    assert!(accepts(
        "        if item > 0 {\n            push(out, item)\n        } else {\n            out\n        }"
    ));
}

/// The same through a `match`, which `std/list` also writes.
#[test]
fn a_match_arm_that_hands_the_accumulator_on_is_accepted() {
    assert!(accepts(
        "        match item {\n            0 => out,\n            _ => push(out, item),\n        }"
    ));
}

/// Both branches pushing is fine, and is not the same as handing on.
///
/// This is the case that tells `handed_on` apart from a count of branches:
/// neither branch is the bare name, so nothing is handed on here, and a rule
/// that counted branches instead would make the arithmetic come out wrong.
#[test]
fn both_branches_pushing_is_accepted() {
    assert!(accepts(
        "        if item > 0 {\n            push(out, item)\n        } else {\n            push(out, 0)\n        }"
    ));
}

/// Growing by two in a turn is refused, which is `intersperse`.
///
/// This is the half of the rule that was learned rather than designed. The
/// mentions are all pushes, so the first half alone accepts it, and what comes
/// out of the turn is a copy that was never given room.
#[test]
fn pushing_twice_in_a_turn_is_refused() {
    assert!(!accepts("        push(push(out, item), item)"));
}

/// A turn whose value is something else entirely.
#[test]
fn handing_back_a_different_list_is_refused() {
    assert!(!accepts("        push(items, item)"));
}

/// A turn that does not mention the accumulator at all.
#[test]
fn never_mentioning_the_accumulator_is_refused() {
    assert!(!accepts("        items"));
}

// Every remaining case is a place the accumulator could be kept. The walk ends
// in a push each time, so the first half of the rule would hold if the mention
// above it were not seen, and each of these asks that it is.

#[test]
fn binding_the_accumulator_to_a_name_is_refused() {
    assert!(!accepts("        let saved = out\n        push(out, item)"));
}

#[test]
fn reading_a_field_of_the_accumulator_is_refused() {
    assert!(!accepts(
        "        let held = out.left\n        push(out, item)"
    ));
}

#[test]
fn putting_the_accumulator_in_a_list_is_refused() {
    assert!(!accepts(
        "        let held = [out]\n        push(out, item)"
    ));
}

#[test]
fn putting_the_accumulator_in_a_record_is_refused() {
    assert!(!accepts(
        "        let held = Held { value: out }\n        push(out, item)"
    ));
}

#[test]
fn passing_the_accumulator_to_something_else_is_refused() {
    assert!(!accepts(
        "        let held = keep(out)\n        push(out, item)"
    ));
}

#[test]
fn the_accumulator_under_a_unary_is_refused() {
    assert!(!accepts("        let held = !out\n        push(out, item)"));
}

#[test]
fn the_accumulator_beside_a_binary_is_refused() {
    assert!(!accepts(
        "        let held = out == items\n        push(out, item)"
    ));
}

#[test]
fn the_accumulator_before_a_question_mark_is_refused() {
    assert!(!accepts("        let held = out?\n        push(out, item)"));
}

#[test]
fn walking_the_accumulator_again_is_refused() {
    assert!(!accepts(
        "        let held = for other in out with inner = [] {\n            push(inner, other)\n        }\n        push(out, item)"
    ));
}

#[test]
fn a_closure_capturing_the_accumulator_is_refused() {
    assert!(!accepts(
        "        let held = |n: Int| out\n        push(out, item)"
    ));
}

#[test]
fn the_accumulator_under_a_handler_is_refused() {
    assert!(!accepts(
        "        let held = with Held { count: 0 } {\n            out\n        }\n        push(out, item)"
    ));
}
