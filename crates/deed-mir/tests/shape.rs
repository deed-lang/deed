//! What the rules in `deed_mir` refuse, asked one shape at a time.
//!
//! `crates/deed-driver/tests/walks.rs` counts how much of the library they
//! accept, which is the measurement the change rests on, but a corpus only
//! contains the shapes somebody happened to write. The dangerous direction is
//! the other one: a body that keeps the accumulator somewhere and is accepted
//! anyway compiles without complaint and answers differently, so every place
//! a name can be kept needs a case here whether or not the library has one.
//!
//! See `design/decisions/2026-08-04-a-walk-that-only-pushes.md` and
//! `design/decisions/2026-08-04-a-walk-that-pushes-into-a-record.md`.

use deed_ast::{Accumulator, Block, Expr, Item};
use deed_mir::{only_pushes, pushed_fields};

/// The one walk in a module written around a body.
fn walk(source: &str) -> (Accumulator, Block) {
    let mut sources = deed_diagnostics::SourceMap::new();
    let file = sources.add("shape.deed".to_string(), source.to_string());
    let lexed = deed_lexer::tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "this body should lex:\n{source}");
    let parsed = deed_parser::parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "this body should parse:\n{source}");

    let Some(Item::Function(decl)) = parsed.module.items.last() else {
        panic!("the module ends in a function");
    };
    let Some(Expr::For {
        accumulator: Some(accumulator),
        body,
        ..
    }) = decl.body.tail.as_deref()
    else {
        panic!("the function's value should be a walk carrying an accumulator");
    };
    (accumulator.clone(), body.clone())
}

/// Whether the rule accepts a walk whose body is this.
fn accepts(body: &str) -> bool {
    let source = format!(
        "module a\n\nfn f(items: List<Int>) -> List<Int> {{\n    for item in items with out = [] {{\n{body}\n    }}\n}}\n"
    );
    let (accumulator, body) = walk(&source);
    only_pushes(&accumulator.name.name, &body)
}

/// Which fields of a record accumulator the rule builds in place.
fn built(init: &str, body: &str) -> Vec<String> {
    let source = format!(
        "module a\n\n\
         record Parts {{\n    kept: List<Int>,\n    rest: List<Int>,\n}}\n\n\
         fn keep(held: Parts) -> Int {{\n    length(held.kept)\n}}\n\n\
         fn seen(xs: List<Int>) -> Int {{\n    length(xs)\n}}\n\n\
         fn f(items: List<Int>) -> Parts {{\n    for item in items with out = {init} {{\n{body}\n    }}\n}}\n"
    );
    let (accumulator, body) = walk(&source);
    pushed_fields(&accumulator.name.name, &accumulator.init, &body)
}

/// The record every case below starts from.
const EMPTY: &str = "Parts { kept: [], rest: [] }";

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

/// And when the arm is written as a block rather than an expression.
#[test]
fn a_match_arm_block_that_hands_the_accumulator_on_is_accepted() {
    assert!(accepts(
        "        match item {\n            0 => {\n                out\n            },\n            _ => push(out, item),\n        }"
    ));
}

/// An `else if` chain hands on from the branch that does, not from the chain.
#[test]
fn an_else_if_that_hands_the_accumulator_on_is_accepted() {
    assert!(accepts(
        "        if item > 0 {\n            push(out, item)\n        } else if item < 0 {\n            out\n        } else {\n            out\n        }"
    ));
}

/// A block arm that pushes is not an arm that hands the accumulator on.
///
/// The pair of this and the one above is what tells the two apart: a rule that
/// counted block arms rather than block arms whose value is the bare name
/// would accept the one above and refuse this.
#[test]
fn a_match_arm_block_that_pushes_is_accepted() {
    assert!(accepts(
        "        match item {\n            0 => {\n                push(out, 0)\n            },\n            _ => push(out, item),\n        }"
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

/// A second push in a turn, away from the value the turn hands back.
///
/// This is `intersperse` again with the two pushes written apart rather than
/// nested, and it is what the rule missed when it counted pushes anywhere
/// rather than asking about the value of a path. The turn grows the list by
/// two while the room reserved is one a turn, so the walk writes past the end
/// of what it was given and answers with a list of the wrong length. The
/// compiled half of it is in `crates/deed-driver/tests/agreement.rs`.
#[test]
fn pushing_away_from_the_value_of_a_turn_is_refused() {
    assert!(!accepts(
        "        let ahead = push(out, item)\n        let _ = length(ahead)\n        push(out, item)"
    ));
}

/// A branch that hands the accumulator on somewhere the turn's value is not.
///
/// Handing on is only handing on when it is what the next turn is given.
/// Anywhere else the branch is a value like any other, and whatever reads it
/// is holding the list the walk is about to write into.
#[test]
fn a_branch_that_keeps_the_accumulator_is_refused() {
    assert!(!accepts(
        "        let held = if item > 0 {\n            out\n        } else {\n            out\n        }\n        let _ = length(held)\n        push(out, item)"
    ));
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

// The accumulator that is a record of lists. The record is rebuilt a turn
// either way; what these decide is which of its fields are one block for the
// whole walk rather than one a turn.

/// The shape the rule is for, which is `unzip`.
#[test]
fn a_record_whose_fields_are_only_pushed_onto_is_accepted() {
    assert_eq!(
        built(
            EMPTY,
            "        Parts { kept: push(out.kept, item), rest: push(out.rest, item) }"
        ),
        ["kept", "rest"]
    );
}

/// A field handed on untouched by the turn that pushes the other, which is
/// `partition`.
#[test]
fn a_field_handed_on_by_the_other_branch_is_accepted() {
    assert_eq!(
        built(
            EMPTY,
            "        if item > 0 {\n            Parts { kept: push(out.kept, item), rest: out.rest }\n        } else {\n            Parts { kept: out.kept, rest: push(out.rest, item) }\n        }"
        ),
        ["kept", "rest"]
    );
}

/// A field nothing ever pushes onto is left alone, which is `scan`'s left.
///
/// Reserving room for a field the walk never appends to would allocate the
/// length of the walk to hold nothing.
#[test]
fn a_field_that_is_only_handed_on_is_left_alone() {
    assert_eq!(
        built(
            EMPTY,
            "        Parts { kept: push(out.kept, item), rest: out.rest }"
        ),
        ["kept"]
    );
}

/// A field that does not start empty, which a reserved block does.
#[test]
fn a_field_that_starts_from_something_is_left_alone() {
    assert_eq!(
        built(
            "Parts { kept: items, rest: [] }",
            "        Parts { kept: push(out.kept, item), rest: push(out.rest, item) }"
        ),
        ["rest"]
    );
}

/// A field read anywhere but the two places the rule is about.
#[test]
fn a_field_read_somewhere_else_is_left_alone() {
    assert_eq!(
        built(
            EMPTY,
            "        let held = seen(out.kept)\n        Parts { kept: push(out.kept, held), rest: push(out.rest, item) }"
        ),
        ["rest"]
    );
}

/// A second push into a field, away from the value the turn hands back.
///
/// The same mistake as `intersperse` written through a field: the turn grows
/// that block by two while the room reserved is one a turn.
#[test]
fn pushing_into_a_field_away_from_the_value_of_a_turn_is_left_alone() {
    assert_eq!(
        built(
            EMPTY,
            "        let ahead = push(out.kept, item)\n        let _ = seen(ahead)\n        Parts { kept: push(out.kept, item), rest: push(out.rest, item) }"
        ),
        ["rest"]
    );
}

/// A field given something that is neither the field nor a push onto it.
#[test]
fn a_field_given_a_different_list_is_left_alone() {
    assert_eq!(
        built(
            EMPTY,
            "        Parts { kept: items, rest: push(out.rest, item) }"
        ),
        ["rest"]
    );
}

/// A turn that hands back a record built somewhere else.
///
/// Nothing here knows what that record holds, so nothing here knows that the
/// block the next turn appends to is the block this walk reserved.
#[test]
fn a_turn_whose_value_is_not_a_record_written_here_is_refused() {
    assert!(
        built(
            EMPTY,
            "        let next = Parts { kept: push(out.kept, item), rest: out.rest }\n        next"
        )
        .is_empty()
    );
}

/// Nothing may hold the record itself, however the fields are written.
///
/// A field is reachable through the record, so a rule that only asked about
/// fields would let the record out and the field with it.
#[test]
fn naming_the_record_is_refused() {
    assert!(
        built(
            EMPTY,
            "        let held = out\n        Parts { kept: push(out.kept, item), rest: push(out.rest, item) }"
        )
        .is_empty()
    );
}

#[test]
fn passing_the_record_to_something_else_is_refused() {
    assert!(
        built(
            EMPTY,
            "        let held = keep(out)\n        Parts { kept: push(out.kept, item), rest: push(out.rest, item) }"
        )
        .is_empty()
    );
}

/// An accumulator that is not a record literal at all.
#[test]
fn an_accumulator_that_is_not_a_record_has_no_fields() {
    assert!(built("items", "        out").is_empty());
}
