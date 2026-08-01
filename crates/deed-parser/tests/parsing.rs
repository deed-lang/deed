//! Parser behaviour.
//!
//! The worked example covers the happy path. Everything else here is a way the
//! parser could quietly do the wrong thing, with recovery getting most of the
//! attention because that is what decides whether one mistake costs one round
//! trip or four.

use deed_ast::{BinaryOp, Expr, Item, Outcome, Pattern, Stmt, UnaryOp};
use deed_diagnostics::{Applicability, Diagnostic, SourceMap, render_human};
use deed_lexer::tokenize;
use deed_parser::{Parsed, codes, parse};

fn parse_source(src: &str) -> (SourceMap, Parsed) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);
    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    (sources, parsed)
}

fn parse_ok(src: &str) -> Parsed {
    let (sources, parsed) = parse_source(src);
    if parsed.has_errors() {
        let rendered: Vec<String> = parsed
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect();
        panic!("expected a clean parse:\n{}", rendered.join("\n"));
    }
    parsed
}

fn codes_of(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code).collect()
}

/// Wraps a function body so tests can talk about statements without ceremony.
///
/// The block tail is appended as a final statement, since whether an expression
/// ended up as a tail or a statement is rarely what a test is about.
fn body_of(src: &str) -> Vec<Stmt> {
    let parsed = parse_ok(&format!("module t\n\nfn f() {{\n{src}\n}}\n"));
    match &parsed.module.items[0] {
        Item::Function(f) => {
            let mut stmts = f.body.stmts.clone();
            if let Some(tail) = &f.body.tail {
                stmts.push(Stmt::Expr((**tail).clone()));
            }
            stmts
        }
        other => panic!("expected a function, got {other:?}"),
    }
}

// -- the worked example ----------------------------------------------------

#[test]
fn the_worked_example_parses_cleanly() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.deed");
    let source = std::fs::read_to_string(path).expect("examples/transfer.deed should exist");

    let mut sources = SourceMap::new();
    let file = sources.add("examples/transfer.deed", source);
    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors());

    let parsed = parse(file, &lexed.tokens);
    if parsed.has_errors() {
        let rendered: Vec<String> = parsed
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect();
        panic!(
            "the worked example should parse cleanly:\n{}",
            rendered.join("\n")
        );
    }

    let module = &parsed.module;
    assert_eq!(
        module.name.as_ref().unwrap().to_string_path(),
        "payments/transfer"
    );
    // The example imports nothing. Everything it needs is declared in it,
    // which is the only way the later passes can check any of it.
    assert!(module.uses.is_empty());

    let functions = module
        .items
        .iter()
        .filter(|i| matches!(i, Item::Function(_)))
        .count();
    let tests = module
        .items
        .iter()
        .filter(|i| matches!(i, Item::Test(_)))
        .count();
    assert!(functions >= 2);
    assert!(tests >= 2);
}

#[test]
fn the_worked_example_contract_is_complete() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.deed");
    let source = std::fs::read_to_string(path).unwrap();
    let mut sources = SourceMap::new();
    let file = sources.add("transfer.deed", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);

    let function = parsed
        .module
        .items
        .iter()
        .find_map(|i| match i {
            Item::Function(f) => Some(f),
            _ => None,
        })
        .expect("transfer should be there");

    assert_eq!(function.sig.name.name, "transfer");
    assert_eq!(function.sig.params.len(), 3);
    assert!(!function.contract.is_pure());

    assert_eq!(function.contract.requires.len(), 1);
    assert_eq!(function.contract.uses.len(), 3);
    assert!(function.contract.ensures.len() >= 4);

    let effects: Vec<String> = function
        .contract
        .uses
        .iter()
        .map(|e| match &e.operation {
            Some(op) => format!("{}.{}", e.effect.name, op.name),
            None => e.effect.name.clone(),
        })
        .collect();
    assert_eq!(effects, ["Ledger.balance", "Ledger.post", "Audit.append"]);

    let outcomes: Vec<Outcome> = function
        .contract
        .ensures
        .iter()
        .map(|e| e.outcome)
        .collect();
    // The success obligations come first and the failure one last, which is
    // how the clause reads in the file.
    assert_eq!(outcomes.last(), Some(&Outcome::Err));
    assert!(outcomes.iter().filter(|o| **o == Outcome::Ok).count() >= 3);
}

// -- declarations ----------------------------------------------------------

#[test]
fn imports_are_named_with_no_wildcard_form() {
    let parsed = parse_ok("module a\n\nuse std/result.{Result, ok, err}\nuse ledger.{Ledger}\n");
    assert_eq!(parsed.module.uses.len(), 2);
    assert_eq!(parsed.module.uses[0].path.to_string_path(), "std/result");
    let names: Vec<&str> = parsed.module.uses[0]
        .names
        .iter()
        .map(|n| n.name.as_str())
        .collect();
    assert_eq!(names, ["Result", "ok", "err"]);
}

#[test]
fn a_module_may_declare_its_edition() {
    let parsed = parse_ok("module a edition 2025\n\nfn f() -> Int { 0 }\n");
    assert_eq!(parsed.module.edition.as_ref().map(|e| e.year), Some(2025));
}

#[test]
fn a_use_semicolon_is_edition_gated() {
    let (_, parsed_2024) = parse_source("module a edition 2024\n\nuse b.{Thing};\n");
    assert!(
        codes_of(&parsed_2024.diagnostics).contains(&codes::UNEXPECTED_TOKEN),
        "edition 2024 should reject `use ...;`"
    );

    let parsed_2025 = parse_ok("module a edition 2025\n\nuse b.{Thing};\n");
    assert_eq!(parsed_2025.module.uses.len(), 1);
}

#[test]
fn an_edition_nobody_declared_is_named_rather_than_guessed_at() {
    let (_, parsed) = parse_source("module a edition 2099\n\nfn f() -> Int { 0 }\n");
    assert!(
        codes_of(&parsed.diagnostics).contains(&codes::UNKNOWN_EDITION),
        "edition 2099 does not exist and should be refused by name"
    );
}

#[test]
fn a_refinement_is_part_of_the_type_alias() {
    let parsed = parse_ok("module a\n\ntype Positive = Int where value > 0\n");
    match &parsed.module.items[0] {
        Item::TypeAlias(alias) => {
            assert_eq!(alias.name.name, "Positive");
            assert!(matches!(
                alias.refinement,
                Some(Expr::Binary {
                    op: BinaryOp::Gt,
                    ..
                })
            ));
        }
        other => panic!("expected a type alias, got {other:?}"),
    }
}

#[test]
fn a_deprecation_declaration_names_the_old_and_new_items() {
    let parsed =
        parse_ok("module a\n\ndeprecated old_name -> new_name\nfn old_name() -> Int { 0 }\n");
    match &parsed.module.items[0] {
        Item::Deprecate(decl) => {
            assert_eq!(decl.old.name, "old_name");
            assert_eq!(decl.new.name, "new_name");
        }
        other => panic!("expected a deprecation declaration, got {other:?}"),
    }
}

#[test]
fn a_choice_variant_may_or_may_not_carry_fields() {
    let parsed = parse_ok(
        "module a\n\nchoice E {\n  WithFields { available: Money },\n  Bare,\n  Empty {},\n}\n",
    );
    match &parsed.module.items[0] {
        Item::Choice(choice) => {
            assert_eq!(choice.variants.len(), 3);
            assert_eq!(choice.variants[0].fields.as_ref().unwrap().len(), 1);
            assert!(choice.variants[1].fields.is_none());
            assert_eq!(choice.variants[2].fields.as_ref().unwrap().len(), 0);
        }
        other => panic!("expected a choice, got {other:?}"),
    }
}

#[test]
fn effects_and_handlers_parse() {
    let parsed = parse_ok(
        "module a\n\
         \n\
         effect Ledger {\n\
         \x20   fn balance(account: AccountId) -> Money\n\
         \x20   fn post(entry: Entry) -> Result<(), LedgerError>\n\
         }\n\
         \n\
         handler InMemory implements Ledger {\n\
         \x20   state accounts: Map<AccountId, Money>\n\
         \n\
         \x20   fn balance(account) -> Money { accounts.get(account) }\n\
         }\n",
    );

    match &parsed.module.items[0] {
        Item::Effect(effect) => {
            assert_eq!(effect.name.name, "Ledger");
            assert_eq!(effect.operations.len(), 2);
        }
        other => panic!("expected an effect, got {other:?}"),
    }
    match &parsed.module.items[1] {
        Item::Handler(handler) => {
            assert_eq!(handler.effect.name, "Ledger");
            assert_eq!(handler.state.len(), 1);
            assert_eq!(handler.operations.len(), 1);
            // Handler operations take their types from the effect.
            assert!(handler.operations[0].sig.params[0].ty.is_none());
        }
        other => panic!("expected a handler, got {other:?}"),
    }
}

#[test]
fn state_is_a_name_everywhere_except_at_the_head_of_a_handler_member() {
    // `state` means something in one position, where the only alternative is
    // `fn`. Reserving it for the whole language cost a name programs want:
    // this one is the accumulator of a fold.
    let parsed = parse_ok(
        "module a\n\
         \n\
         handler Counted implements Log {\n\
         \x20   state seen: Int\n\
         }\n\
         \n\
         fn total(ns: List<Int>) -> Int {\n\
         \x20   for n in ns with state = 0 { state + n }\n\
         }\n",
    );

    match &parsed.module.items[0] {
        Item::Handler(handler) => {
            assert_eq!(handler.state.len(), 1);
            assert_eq!(handler.state[0].name.name, "seen");
        }
        other => panic!("expected a handler, got {other:?}"),
    }
    assert!(matches!(&parsed.module.items[1], Item::Function(_)));
}

#[test]
fn an_absent_uses_clause_means_pure() {
    let parsed = parse_ok("module a\n\nfn double(n: Int) -> Int { n * 2 }\n");
    match &parsed.module.items[0] {
        Item::Function(f) => {
            assert!(f.contract.is_pure());
            assert!(f.contract.is_empty());
            assert!(f.contract.span.is_none());
        }
        other => panic!("expected a function, got {other:?}"),
    }
}

#[test]
fn nested_generics_need_no_shift_handling() {
    // There is no `>>` token, so `Map<K, Vec<V>>` closes with two `>` tokens.
    parse_ok("module a\n\nfn f(m: Map<AccountId, Vec<Money>>) -> Int { 0 }\n");
}

#[test]
fn a_parameter_without_a_type_is_reported() {
    // P5: nothing implicit crosses a boundary, and a parameter is the
    // boundary. Left alone, the type became unknown, unknown agrees with
    // everything, and a closure could carry any effect through it.
    let (_, parsed) = parse_source("module a\n\nfn f(n) -> Int { n }\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::MISSING_PARAMETER_TYPE]
    );
}

#[test]
fn the_rest_of_the_file_still_parses_after_one() {
    // Reported rather than refused. A parser that gave up here would show one
    // mistake per run when the author wants to see all of them.
    let (_, parsed) = parse_source("module a\n\nfn f(n) -> Int { n }\n\nfn g(m) -> Int { m }\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::MISSING_PARAMETER_TYPE, codes::MISSING_PARAMETER_TYPE]
    );
    assert_eq!(parsed.module.items.len(), 2);
}

#[test]
fn an_effect_operation_needs_its_types_too() {
    // The declaration is the only place they could be written.
    let (_, parsed) = parse_source("module a\n\neffect E {\n    fn go(n) -> Int\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::MISSING_PARAMETER_TYPE]
    );
}

#[test]
fn a_handler_operation_does_not() {
    // The effect it implements already said what the types are, and making the
    // handler repeat them would be redundancy nothing checks.
    parse_ok(
        "module a\n\n\
         effect E {\n    fn go(n: Int) -> Int\n}\n\n\
         handler H implements E {\n    fn go(n) -> Int { n }\n}\n",
    );
}

#[test]
fn a_closure_parameter_needs_one_too() {
    // Briefly exempt, on the grounds that a closure cannot leave the function
    // that wrote it so its parameters are not a boundary anyone reviews. True,
    // and a different claim from "may be unchecked": with no type they were
    // the unknown type and the body was checked against nothing.
    let (_, parsed) = parse_source(
        "module a\n\nfn f(a: Int, b: Int) -> Int {\n    let add = |x, y| { x + y }\n    add(a, b)\n}\n",
    );
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::MISSING_PARAMETER_TYPE, codes::MISSING_PARAMETER_TYPE]
    );
}

// -- positional variants ---------------------------------------------------

/// The first thing somebody arriving from a language with tuple variants
/// writes, and it is refused. It used to be refused as "expected `}`", which
/// reads as a missing brace in a line that has none.
#[test]
fn a_variant_written_by_position_says_so() {
    let (sources, parsed) =
        parse_source("module a\n\nchoice Shape {\n    Nothing,\n    Circle(Int),\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::POSITIONAL_VARIANT]
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(
        text.contains("`Circle` carries its payload by position"),
        "{text}"
    );
    assert!(text.contains("`Variant { field: Type }`"), "{text}");
    // Where the argument about whether it should be refused at all lives.
    assert!(text.contains("`ok` and `err`"), "{text}");
}

/// The payload is skipped rather than left for the next rule to trip over, so
/// one of these is one error and the reader gets the rest of the file.
#[test]
fn the_declaration_survives_and_keeps_its_other_variants() {
    let (_, parsed) = parse_source(
        "module a\n\nchoice Shape {\n    Nothing,\n    Circle(Int),\n    Named { label: String },\n}\n\nfn f() -> Int { 0 }\n",
    );
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::POSITIONAL_VARIANT]
    );

    let Item::Choice(decl) = &parsed.module.items[0] else {
        panic!("the choice should still be there");
    };
    let names: Vec<&str> = decl
        .variants
        .iter()
        .map(|variant| variant.name.name.as_str())
        .collect();
    assert_eq!(names, vec!["Nothing", "Circle", "Named"]);
    // And the declaration after it, which a cascade would have swallowed.
    assert!(matches!(parsed.module.items[1], Item::Function(_)));
}

/// Each one is its own mistake. Reporting the first and giving up would mean
/// a second pass for somebody translating a type over from another language.
#[test]
fn every_positional_variant_is_reported() {
    let (_, parsed) =
        parse_source("module a\n\nchoice Shape {\n    Circle(Int),\n    Rect(Int, Int),\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::POSITIONAL_VARIANT, codes::POSITIONAL_VARIANT]
    );
}

/// A payload with brackets of its own, which a scan looking for the first `)`
/// would have stopped in the middle of.
#[test]
fn a_nested_payload_is_skipped_whole() {
    let (_, parsed) =
        parse_source("module a\n\nchoice C {\n    Run(Fn(Int) -> Int),\n    After,\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::POSITIONAL_VARIANT]
    );

    let Item::Choice(decl) = &parsed.module.items[0] else {
        panic!("the choice should still be there");
    };
    let names: Vec<&str> = decl
        .variants
        .iter()
        .map(|variant| variant.name.name.as_str())
        .collect();
    assert_eq!(names, vec!["Run", "After"]);
}

#[test]
fn a_variant_with_named_fields_is_untouched() {
    parse_ok("module a\n\nchoice C {\n    One,\n    Two { x: Int, y: Int },\n}\n");
}

// -- words from other languages --------------------------------------------

/// The first thing anybody writes in a new language is the thing they wrote in
/// the last one. `struct` used to be answered with the list of all seven
/// declaration forms, which is correct and is a slower way to say `record`.
#[test]
fn struct_is_answered_with_record() {
    let (sources, parsed) = parse_source("module a\n\nstruct Point {\n    x: Int,\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics)[0],
        codes::EXPECTED_DECLARATION
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("this language spells it `record`"), "{text}");

    // Machine applicable, so `deed fix` repairs the file rather than offering
    // to. There is one thing `struct` can mean here and this is it.
    let fix = parsed.diagnostics[0].fix.as_ref().expect("a fix");
    assert_eq!(fix.edits[0].replacement, "record");
    assert_eq!(
        fix.applicability,
        deed_diagnostics::Applicability::MachineApplicable
    );
}

#[test]
fn the_other_renamings_are_answered_the_same_way() {
    for (theirs, ours) in [
        ("class", "record"),
        ("enum", "choice"),
        ("import", "use"),
        ("function", "fn"),
    ] {
        let source = format!("module a\n\n{theirs} Thing {{\n}}\n");
        let (sources, parsed) = parse_source(&source);
        let text = render_human(&sources, &parsed.diagnostics[0]);
        assert!(
            text.contains(&format!("this language spells it `{ours}`")),
            "`{theirs}` should point at `{ours}`: {text}"
        );
    }
}

/// A word for a thing the language decided not to have gets the decision
/// rather than a replacement, because there is nothing to replace it with.
#[test]
fn pub_is_answered_with_the_reason_there_is_no_such_word() {
    let (sources, parsed) = parse_source("module a\n\npub fn f() -> Int { 0 }\n");
    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("every declaration is exported"), "{text}");
    assert!(text.contains("no wildcard imports"), "{text}");
    assert!(
        parsed.diagnostics[0].fix.is_none(),
        "there is no word to put in its place, so offering one would be inventing a change"
    );
}

/// Everything else still gets the list. A name that is nobody's keyword is
/// somebody's typo, and the seven forms are the answer to that.
#[test]
fn a_word_from_nowhere_still_gets_the_list() {
    let (sources, parsed) = parse_source("module a\n\nwidget Thing {\n}\n");
    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("`record`, `choice`, `effect`"), "{text}");
    assert!(!text.contains("spells it"), "{text}");
}

/// `let mut n = 1` used to take `mut` as the name and produce six messages: an
/// unused binding called `mut` offering to rename it `_mut`, a missing `=`,
/// `n` not found twice, a stray `=`, and a `1` going nowhere. Not one of them
/// mentioned the word that was actually written.
#[test]
fn a_word_in_front_of_a_let_name_is_one_message_and_not_six() {
    let (sources, parsed) =
        parse_source("module a\n\nfn f() -> Int {\n    let mut n = 1\n    n\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::NO_BINDING_MODIFIER]
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("there is no `mut`"), "{text}");
    assert!(text.contains("handler's `state` field"), "{text}");
    assert!(text.contains("with sum = 0"), "{text}");

    // The name after it stands in, so the rest of the function is read as
    // written rather than collapsing into names that cannot be found.
    let fix = parsed.diagnostics[0].fix.as_ref().expect("a fix");
    assert_eq!(fix.edits[0].replacement, "");
}

#[test]
fn a_word_that_asks_for_what_a_let_already_gives_says_so() {
    let (sources, parsed) =
        parse_source("module a\n\nfn f() -> Int {\n    let mutable n = 1\n    n\n}\n");
    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("there is no `mutable`"), "{text}");
}

/// Two names in a row is not a pattern, which is what makes the reading above
/// safe, and is the same fact `assert refuses f(x)` rests on. Anything that
/// was already a `let` has to keep parsing as one.
#[test]
fn an_ordinary_let_is_not_read_as_a_word_in_front_of_a_name() {
    parse_ok("module a\n\nfn f() -> Int {\n    let n = 1\n    n\n}\n");
    // A binding that happens to be called `mut` is still a binding, because
    // what follows it is `=` and not another name.
    parse_ok("module a\n\nfn f() -> Int {\n    let mut = 1\n    mut\n}\n");
    parse_ok("module a\n\nfn f() -> Int {\n    let n: Int = 1\n    n\n}\n");
}

#[test]
fn a_closure_with_typed_parameters_parses() {
    parse_ok(
        "module a\n\nfn f(a: Int, b: Int) -> Int {\n    let add = |x: Int, y: Int| { x + y }\n    add(a, b)\n}\n",
    );
}

// -- a binding written without `let` ---------------------------------------
//
// `var n = 1` and `Int n = 1` used to be read as the two halves they look
// like, a name on its own and an assignment to another name, so the reader was
// told twice that a name could not be found and never that the line wanted a
// `let`.

#[test]
fn another_languages_binding_keyword_is_answered_with_let() {
    let (sources, parsed) = parse_source("module a\n\nfn f() -> Int {\n    var n = 1\n    n\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::BINDING_WITHOUT_LET]
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("there is no `var`"), "{text}");
    assert!(text.contains("binds its name once"), "{text}");

    let fix = parsed.diagnostics[0].fix.as_ref().expect("a fix");
    assert_eq!(fix.edits[0].replacement, "let");
}

/// The word that asked for something the language refuses hears why, and the
/// word that asked for what a `let` already is does not need to.
#[test]
fn only_the_word_that_asked_for_a_mutable_one_hears_about_state() {
    let (sources, parsed) = parse_source("module a\n\nfn f() -> Int {\n    var n = 1\n    n\n}\n");
    let mutable = render_human(&sources, &parsed.diagnostics[0]);
    assert!(mutable.contains("handler's `state` field"), "{mutable}");
    assert!(mutable.contains("with sum = 0"), "{mutable}");

    for word in ["const", "val"] {
        let source = format!("module a\n\nfn f() -> Int {{\n    {word} n = 1\n    n\n}}\n");
        let (sources, parsed) = parse_source(&source);
        let text = render_human(&sources, &parsed.diagnostics[0]);
        assert!(text.contains(&format!("there is no `{word}`")), "{text}");
        assert!(
            !text.contains("handler's `state` field"),
            "`{word}` is asking for what a `let` already is: {text}"
        );
    }
}

#[test]
fn a_type_in_front_of_the_name_is_told_where_the_type_goes() {
    let (sources, parsed) = parse_source("module a\n\nfn f() -> Int {\n    Int k = 3\n    k\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::BINDING_WITHOUT_LET]
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("`Int` is the type of `k`"), "{text}");
    assert!(text.contains("let name: Type = value"), "{text}");

    let fix = parsed.diagnostics[0].fix.as_ref().expect("a fix");
    assert_eq!(fix.edits[0].replacement, "let k: Int");
}

/// The statement goes on being read as the `let` it was meant to be, so one
/// mistake stays one message rather than taking the lines below it down too.
/// The type keeps its place, which is the whole reason for reporting it here
/// rather than dropping it.
#[test]
fn the_line_is_still_read_as_the_binding_it_meant() {
    for (source, typed) in [
        (
            "module a\n\nfn f() -> Int {\n    var n = 1\n    n\n}\n",
            false,
        ),
        (
            "module a\n\nfn f() -> Int {\n    Int n = 1\n    n\n}\n",
            true,
        ),
    ] {
        let (_, parsed) = parse_source(source);
        let Item::Function(function) = &parsed.module.items[0] else {
            panic!("expected a function");
        };
        match &function.body.stmts[0] {
            Stmt::Let { pattern, ty, .. } => {
                assert!(
                    matches!(pattern, Pattern::Path { segments, .. } if segments[0].name == "n"),
                    "the name has to be bound or the lines below it cannot find it"
                );
                assert_eq!(ty.is_some(), typed, "{source}");
            }
            other => panic!("expected a `let`, got {other:?}"),
        }
    }
}

/// The line break is the other half of what makes the reading safe. An
/// expression and an assignment on two lines is a program.
#[test]
fn a_name_on_the_line_below_is_not_part_of_the_line_above() {
    // `Total` is capitalised, so this would be read as a type in front of a
    // name if the parser only counted tokens.
    parse_ok("module a\n\nfn f(m: Int) -> () {\n    Total\n    n = m\n}\n");
}

/// A word that is neither of the two readings keeps the answer it has today.
/// The shape is wrong either way, but guessing `let` at anything would put a
/// word in somebody's file on no evidence.
#[test]
fn a_word_that_is_neither_is_left_alone() {
    let (_, parsed) = parse_source("module a\n\nfn f() -> Int {\n    widget n = 1\n    n\n}\n");
    assert!(
        !codes_of(&parsed.diagnostics).contains(&codes::BINDING_WITHOUT_LET),
        "{:?}",
        parsed.diagnostics
    );
}

/// At the top level the same word gets a different answer, because there the
/// reading is not a binding at all: a file holds declarations.
#[test]
fn var_at_the_top_level_still_gets_the_declaration_answer() {
    let (sources, parsed) = parse_source("module a\n\nvar n = 1\n");
    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("a file holds declarations"), "{text}");
}

// -- a range -----------------------------------------------------------------
//
// `for i in 0..10` is what anybody writes who wants to count, and the dots
// used to be left where they were. The `for` went looking for its block, found
// a dot, and the rest of the file came apart behind it.

#[test]
fn a_range_is_one_message_rather_than_six() {
    let (sources, parsed) =
        parse_source("module a\n\nfn f() -> () {\n    for i in 0..10 {\n        i\n    }\n}\n");
    assert_eq!(codes_of(&parsed.diagnostics), vec![codes::NO_RANGE]);

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("there is no range"), "{text}");
    assert!(text.contains("walks a list that already exists"), "{text}");
    assert!(
        text.contains("`repeat(value, count)` makes the list"),
        "{text}"
    );
    assert!(text.contains("for item at i in"), "{text}");
}

/// The bound is read and thrown away, so the block after it is still a block
/// and the declaration after that is still a declaration. This is the whole
/// reason for reading the shape rather than reporting the first dot.
#[test]
fn what_comes_after_a_range_still_parses() {
    let (_, parsed) = parse_source(
        "module a\n\nfn f() -> () {\n    for i in 0..10 {\n        i\n    }\n}\n\nfn g() -> Int { 1 }\n",
    );
    assert_eq!(codes_of(&parsed.diagnostics), vec![codes::NO_RANGE]);
    assert_eq!(parsed.module.items.len(), 2, "{:?}", parsed.module.items);
}

#[test]
fn an_inclusive_range_is_the_same_mistake_with_one_more_character() {
    let (_, parsed) =
        parse_source("module a\n\nfn f() -> () {\n    for i in 0..=10 {\n        i\n    }\n}\n");
    assert_eq!(codes_of(&parsed.diagnostics), vec![codes::NO_RANGE]);
}

/// Nothing about this is specific to a `for`. The dots are the mistake
/// wherever they are written.
#[test]
fn a_range_anywhere_else_gets_the_same_answer() {
    let (_, parsed) =
        parse_source("module a\n\nfn f(n: Int) -> Int {\n    let r = 0..n\n    n\n}\n");
    assert_eq!(codes_of(&parsed.diagnostics), vec![codes::NO_RANGE]);
}

/// Two dots in a row are only ever this, because a field access has a name
/// between them. That is what makes the reading safe, so it has a test.
#[test]
fn a_chain_of_field_accesses_is_not_a_range() {
    parse_ok("module a\n\nfn f(r: R) -> Int {\n    r.inner.n\n}\n");
}

// -- a cast ------------------------------------------------------------------
//
// `as` is an ordinary name, so `n as String` used to be answered with "cannot
// find `as` in this scope", which is true of a word nobody wrote as a name.

#[test]
fn a_cast_says_the_conversion_is_a_call() {
    let (sources, parsed) =
        parse_source("module a\n\nfn f(n: Int) -> String {\n    n as String\n}\n");
    assert_eq!(codes_of(&parsed.diagnostics), vec![codes::NO_CAST]);

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("there is no cast"), "{text}");
    assert!(text.contains("whether it can fail"), "{text}");
    assert!(text.contains("to_string(n)"), "{text}");
}

/// Wrapping the value is an insertion in front of it and a replacement behind
/// it, which is the first fix in this compiler that takes two edits.
#[test]
fn the_fix_for_a_cast_wraps_the_value() {
    let (_, parsed) = parse_source("module a\n\nfn f(n: Int) -> String {\n    n as String\n}\n");
    let fix = parsed.diagnostics[0].fix.as_ref().expect("a fix");
    assert_eq!(fix.edits.len(), 2);
    assert_eq!(fix.edits[0].replacement, "to_string(");
    assert_eq!(fix.edits[1].replacement, ")");
}

/// `to_int` gives a `Result`, so what to do with it is the reader's decision
/// and not a rewrite anyone can make on their behalf.
#[test]
fn the_conversion_that_can_fail_is_offered_rather_than_applied() {
    let (sources, parsed) = parse_source("module a\n\nfn f(s: String) -> Int {\n    s as Int\n}\n");
    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("to_int(s)"), "{text}");
    assert!(text.contains("gives a `Result`"), "{text}");

    let fix = parsed.diagnostics[0].fix.as_ref().expect("a fix");
    assert_eq!(fix.applicability, Applicability::MaybeIncorrect);
}

/// A target with no conversion to point at gets the list and no fix, because
/// inventing a call nobody wrote is worse than saying there is not one.
#[test]
fn a_target_with_no_conversion_gets_no_fix() {
    let (sources, parsed) = parse_source("module a\n\nfn f(n: Int) -> Bool {\n    n as Bool\n}\n");
    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("the conversions the prelude has"), "{text}");
    assert!(parsed.diagnostics[0].fix.is_none(), "{text}");
}

/// The type is parsed rather than skipped, so an argument list does not turn
/// into comparisons that never close.
#[test]
fn a_cast_to_a_generic_type_is_still_one_message() {
    let (_, parsed) =
        parse_source("module a\n\nfn f(n: Int) -> Int {\n    let xs = n as List<Int>\n    n\n}\n");
    assert_eq!(codes_of(&parsed.diagnostics), vec![codes::NO_CAST]);
}

/// `as` stays an ordinary name. Only a value followed by two names on one line
/// is the cast, and nothing else in the grammar is that.
#[test]
fn as_is_still_a_name() {
    parse_ok("module a\n\nfn f(as: Int) -> Int {\n    as + 1\n}\n");
    parse_ok("module a\n\nfn f() -> Int {\n    let as = 1\n    as\n}\n");
}

// -- a detached spawn --------------------------------------------------------
//
// `spawn(f())` is what anybody arriving from Go, Rust, or Kotlin writes who
// wants a background task, and the identifier `spawn` followed by `(` on the
// same line is the pattern. Detached spawn is not in this language: a task is
// tied to the block that started it, so there is no child running after the
// parent exits.

#[test]
fn a_detached_spawn_says_there_is_no_such_construct() {
    let (sources, parsed) =
        parse_source("module a\n\nfn f() -> () {}\n\nfn g() -> () {\n    spawn(f())\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::NO_DETACHED_SPAWN]
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("there is no detached spawn"), "{text}");
    assert!(text.contains("tied to the block that started it"), "{text}");
}

/// The argument list is parsed and thrown away, so the statement after
/// `spawn(...)` is still a statement. This is the whole reason for reading
/// the arguments rather than reporting the first token and stopping.
#[test]
fn what_comes_after_a_spawn_still_parses() {
    let (_, parsed) =
        parse_source("module a\n\nfn f() -> () {}\n\nfn g() -> () {\n    spawn(f())\n    f()\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::NO_DETACHED_SPAWN]
    );
}

/// A parent that would return while a child is still running cannot be
/// written: there is no mechanism to start such a child. This test records
/// that the pattern is explicitly refused rather than silently broken.
#[test]
fn a_parent_that_returns_before_its_child_is_refused() {
    let (sources, parsed) = parse_source(
        "module a\n\nfn child() -> () {}\n\nfn parent() -> () {\n    spawn(child())\n}\n",
    );
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::NO_DETACHED_SPAWN]
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(
        text.contains("when concurrency arrives"),
        "the message should point toward the structured alternative: {text}"
    );
}

/// `spawn` stays an ordinary name everywhere the pattern does not apply: as a
/// parameter, a binding, a function name, or alone without a call on the same
/// line. Only `spawn(args)` at statement level on one line is refused.
#[test]
fn spawn_is_still_a_name_outside_the_pattern() {
    parse_ok("module a\n\nfn spawn(n: Int) -> Int {\n    n\n}\n");
    parse_ok("module a\n\nfn f(spawn: Int) -> Int {\n    spawn\n}\n");
    // `spawn` on one line, call on the next line: two separate statements.
    parse_ok(
        "module a\n\nfn spawn() -> () {}\n\nfn g() -> () {\n    let spawn = 1\n    spawn\n}\n",
    );
}

// -- where one statement stops ---------------------------------------------
//
// Statements are separated by nothing, so what ends one is the next token not
// being able to continue it. That works for almost every token and used to
// fail silently for the two that can both start an expression and continue
// one, which is what these are about.

#[test]
fn a_minus_on_the_next_line_starts_a_statement() {
    // `let a = 1` followed by `-2` used to be `let a = 1 - 2`, with the second
    // line gone and nothing saying so.
    let stmts = body_of("let a = 1\n-2");
    assert_eq!(stmts.len(), 2, "{stmts:?}");

    let Stmt::Let { init, .. } = &stmts[0] else {
        panic!("expected a let, got {:?}", stmts[0]);
    };
    assert!(matches!(init, Expr::Int { value: 1, .. }), "{init:?}");

    let Stmt::Expr(Expr::Unary { op, .. }) = &stmts[1] else {
        panic!("expected a negation, got {:?}", stmts[1]);
    };
    assert_eq!(*op, UnaryOp::Neg);
}

#[test]
fn a_parenthesis_on_the_next_line_starts_a_statement() {
    // `let a = g()` followed by `(1 + 2)` used to be a call of the result, and
    // the error landed on `g()` for not being a function.
    let stmts = body_of("let a = g()\n(1 + 2)");
    assert_eq!(stmts.len(), 2, "{stmts:?}");

    let Stmt::Expr(Expr::Binary { op, .. }) = &stmts[1] else {
        panic!("expected the sum on its own, got {:?}", stmts[1]);
    };
    assert_eq!(*op, BinaryOp::Add);
}

#[test]
fn the_same_line_still_continues() {
    // The rule has to leave every shape anybody writes alone, and `deed fmt`
    // never breaks a binary expression or puts a call's parenthesis on a line
    // of its own.
    let stmts = body_of("let a = 1 - 2\nlet b = g()(3)\nlet c = a.b.c");
    assert_eq!(stmts.len(), 3, "{stmts:?}");

    let Stmt::Let { init, .. } = &stmts[0] else {
        panic!("expected a let");
    };
    assert!(matches!(init, Expr::Binary { .. }), "{init:?}");

    let Stmt::Let { init, .. } = &stmts[1] else {
        panic!("expected a let");
    };
    let Expr::Call { callee, .. } = init else {
        panic!("expected a call, got {init:?}");
    };
    assert!(matches!(&**callee, Expr::Call { .. }), "{callee:?}");

    let Stmt::Let { init, .. } = &stmts[2] else {
        panic!("expected a let");
    };
    assert!(matches!(init, Expr::Field { .. }), "{init:?}");
}

#[test]
fn a_call_can_still_spread_its_arguments_over_lines() {
    // The parenthesis stays with the callee, which is where the formatter puts
    // it, and everything inside it is its own expression again.
    let stmts = body_of("let a = f(\n    1,\n    2,\n)");
    let Stmt::Let { init, .. } = &stmts[0] else {
        panic!("expected a let");
    };
    let Expr::Call { args, .. } = init else {
        panic!("expected a call, got {init:?}");
    };
    assert_eq!(args.len(), 2);
}

#[test]
fn a_comment_between_the_lines_does_not_join_them() {
    // The break is measured over the text that was skipped, so a comment with
    // a newline in it counts the way a reader would count it.
    let stmts = body_of("let a = 1\n// why\n-2");
    assert_eq!(stmts.len(), 2, "{stmts:?}");
}

#[test]
fn an_operator_starting_a_line_says_why_it_is_stuck() {
    // The mistake the rule creates. Every language that lets a long sum break
    // over two lines makes this a natural thing to write, and without the note
    // the error says what happened and none of why.
    let (sources, parsed) =
        parse_source("module t\n\nfn f() -> Int {\n    let a = 1\n    * 2\n    a\n}\n");
    assert!(parsed.has_errors());

    let text: Vec<String> = parsed
        .diagnostics
        .iter()
        .map(|d| render_human(&sources, d))
        .collect();
    let text = text.join("\n");
    assert!(
        text.contains("an expression ends at the end of a line"),
        "{text}"
    );
    assert!(
        text.contains("leave the operator on the line above"),
        "{text}"
    );
}

#[test]
fn what_followed_the_operator_is_part_of_the_same_mistake() {
    // The right hand side was meant to belong to it, so taking it here keeps
    // it from becoming a statement of its own and drawing a second complaint
    // from a later pass. One mistake, one diagnostic.
    let (_, parsed) =
        parse_source("module t\n\nfn f() -> Int {\n    let a = 1\n    * 2\n    a\n}\n");
    assert_eq!(codes_of(&parsed.diagnostics), vec![codes::UNEXPECTED_TOKEN]);
}

#[test]
fn an_operator_that_can_start_an_expression_is_left_alone() {
    // `-2` on its own line is a negation and a perfectly good statement, so
    // there is nothing to explain and nothing to swallow.
    let stmts = body_of("let a = 1\n-2\na");
    assert_eq!(stmts.len(), 3, "{stmts:?}");
}

// -- expressions -----------------------------------------------------------

#[test]
fn precedence_binds_multiplication_tighter_than_comparison() {
    let stmts = body_of("let x = 1 + 2 * 3 < 10 && true");
    let Stmt::Let { init, .. } = &stmts[0] else {
        panic!("expected a let");
    };

    // Top level should be `&&`.
    let Expr::Binary {
        op: BinaryOp::And,
        lhs,
        ..
    } = init
    else {
        panic!("expected && at the top, got {init:?}");
    };
    // Then `<`, then `+`, then `*` deepest.
    let Expr::Binary {
        op: BinaryOp::Lt,
        lhs: sum,
        ..
    } = &**lhs
    else {
        panic!("expected < under &&");
    };
    let Expr::Binary {
        op: BinaryOp::Add,
        rhs: product,
        ..
    } = &**sum
    else {
        panic!("expected + under <");
    };
    assert!(matches!(
        &**product,
        Expr::Binary {
            op: BinaryOp::Mul,
            ..
        }
    ));
}

#[test]
fn postfix_chains_compose() {
    let stmts = body_of("let r = transfer(a, b, 40.try).unwrap()?");
    let Stmt::Let { init, .. } = &stmts[0] else {
        panic!("expected a let");
    };
    // `?` outermost, then the call to unwrap, then the field access.
    let Expr::Try { operand, .. } = init else {
        panic!("expected `?` outermost, got {init:?}");
    };
    let Expr::Call { callee, .. } = &**operand else {
        panic!("expected a call under `?`");
    };
    assert!(matches!(&**callee, Expr::Field { .. }));
}

#[test]
fn a_list_literal_takes_a_trailing_comma_or_no_elements() {
    for (src, count) in [
        ("let x = []", 0),
        ("let x = [1]", 1),
        ("let x = [1, 2, 3]", 3),
        ("let x = [1, 2, 3,]", 3),
    ] {
        let stmts = body_of(src);
        let Stmt::Let { init, .. } = &stmts[0] else {
            panic!("expected a let");
        };
        let Expr::List { elements, .. } = init else {
            panic!("expected a list, got {init:?}");
        };
        assert_eq!(elements.len(), count, "in `{src}`");
    }
}

#[test]
fn a_struct_literal_is_allowed_inside_a_list() {
    // The restriction that keeps `if x { }` from reading as a literal is
    // lifted inside brackets, the same as inside an argument list.
    let stmts = body_of("let x = [Point { x: 1 }]");
    let Stmt::Let { init, .. } = &stmts[0] else {
        panic!("expected a let");
    };
    let Expr::List { elements, .. } = init else {
        panic!("expected a list, got {init:?}");
    };
    assert!(matches!(elements[0], Expr::StructLit { .. }));
}

#[test]
fn a_statement_can_start_with_a_list() {
    // Statements have no terminator, so separation relies on nothing being
    // able to continue the line above. `[` was on the list of characters that
    // would break that if anything ever started with one, and now something
    // does. It still holds only because there is no indexing operator.
    let stmts = body_of("let x = f()\n[1, 2]");
    assert_eq!(stmts.len(), 2, "got {stmts:?}");
    assert!(matches!(stmts[1], Stmt::Expr(Expr::List { .. })));
}

#[test]
fn a_for_reads_its_binder_iterable_and_accumulator() {
    let stmts = body_of("let total = for n in numbers with sum = 0 {\n  sum + n\n}");
    let Stmt::Let { init, .. } = &stmts[0] else {
        panic!("expected a let");
    };
    let Expr::For {
        binder,
        accumulator,
        ..
    } = init
    else {
        panic!("expected a for, got {init:?}");
    };
    assert_eq!(binder.name, "n");
    let accumulator = accumulator.as_ref().expect("an accumulator");
    assert_eq!(accumulator.name.name, "sum");
}

#[test]
fn a_for_without_with_has_no_accumulator() {
    let stmts = body_of("for line in lines {\n  f(line)\n}");
    let Stmt::Expr(Expr::For { accumulator, .. }) = &stmts[0] else {
        panic!("expected a for, got {:?}", stmts[0]);
    };
    assert!(accumulator.is_none());
}

#[test]
fn an_assert_can_say_something_is_refused() {
    let stmts = body_of("assert refuses order_of(0)");
    let Stmt::Refuses { subject, .. } = &stmts[0] else {
        panic!("expected a refuses, got {:?}", stmts[0]);
    };
    assert!(matches!(subject, Expr::Call { .. }), "{subject:?}");
}

#[test]
fn refuses_is_still_a_name_when_it_is_called() {
    // The direction this has to fail in. `refuses` is the marker only when an
    // identifier follows it, and no statement could ever have been two names
    // in a row, so a program with a function called `refuses` keeps working.
    let stmts = body_of("assert refuses(0)");
    let Stmt::Assert { condition, .. } = &stmts[0] else {
        panic!("expected an assert, got {:?}", stmts[0]);
    };
    let Expr::Call { callee, .. } = condition else {
        panic!("expected a call, got {condition:?}");
    };
    assert!(matches!(&**callee, Expr::Ident(name) if name.name == "refuses"));
}

#[test]
fn a_for_can_bind_where_in_the_list_it_is() {
    let stmts = body_of("for line at here in lines {\n  f(line, here)\n}");
    let Stmt::Expr(Expr::For { binder, index, .. }) = &stmts[0] else {
        panic!("expected a for, got {:?}", stmts[0]);
    };
    assert_eq!(binder.name, "line");
    assert_eq!(index.as_ref().expect("an index").name, "here");
}

#[test]
fn a_for_without_at_binds_no_index() {
    let stmts = body_of("for line in lines {\n  f(line)\n}");
    let Stmt::Expr(Expr::For { index, .. }) = &stmts[0] else {
        panic!("expected a for, got {:?}", stmts[0]);
    };
    assert!(index.is_none());
}

#[test]
fn at_is_still_a_name_everywhere_else() {
    // The prelude function that indexes a list is called `at`, so reserving
    // the word for this one position would cost a name people already use.
    // The only things that can follow a `for` binder are `at` and `in`.
    let stmts = body_of("let first = at(items, 0)");
    let Stmt::Let { init, .. } = &stmts[0] else {
        panic!("expected a let, got {:?}", stmts[0]);
    };
    let Expr::Call { callee, .. } = init else {
        panic!("expected a call, got {init:?}");
    };
    assert!(matches!(&**callee, Expr::Ident(name) if name.name == "at"));
}

#[test]
fn a_brace_after_a_for_head_is_the_body_not_a_struct_literal() {
    // The same lookahead the `if` condition needs. Without it, `for n in items
    // { ... }` reads `items { ... }` as a literal and the loop loses its body.
    let stmts = body_of("for n in items {\n  f(n)\n}");
    let Stmt::Expr(Expr::For { iterable, .. }) = &stmts[0] else {
        panic!("expected a for, got {:?}", stmts[0]);
    };
    assert!(
        matches!(&**iterable, Expr::Ident(name) if name.name == "items"),
        "the iterable swallowed the body: {iterable:?}"
    );
}

#[test]
fn a_brace_after_an_if_condition_is_a_block_not_a_struct_literal() {
    let stmts = body_of("if available < amount {\n  return err(x)\n}");
    let Stmt::Expr(Expr::If { condition, .. }) = &stmts[0] else {
        panic!("expected an if, got {:?}", stmts[0]);
    };
    assert!(
        matches!(
            &**condition,
            Expr::Binary {
                op: BinaryOp::Lt,
                ..
            }
        ),
        "the condition should stop before the brace, got {condition:?}"
    );
}

#[test]
fn struct_literals_support_the_shorthand_form() {
    let stmts = body_of("let r = Receipt { from, to, amount: value }");
    let Stmt::Let { init, .. } = &stmts[0] else {
        panic!("expected a let");
    };
    let Expr::StructLit { fields, .. } = init else {
        panic!("expected a struct literal, got {init:?}");
    };
    assert_eq!(fields.len(), 3);
    assert!(fields[0].value.is_none(), "`from` should be shorthand");
    assert!(fields[2].value.is_some(), "`amount: value` should not be");
}

#[test]
fn old_and_unchanged_are_their_own_nodes() {
    let parsed = parse_ok(
        "module a\n\n\
         fn f() -> Int\n\
         \x20 ensures\n\
         \x20   ok  => balance(x) == old(balance(x)) - 1,\n\
         \x20   err => unchanged(Ledger),\n\
         { 0 }\n",
    );
    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };

    let Expr::Binary { rhs, .. } = &function.contract.ensures[0].condition else {
        panic!("expected a comparison");
    };
    let Expr::Binary { lhs, .. } = &**rhs else {
        panic!("expected a subtraction");
    };
    assert!(
        matches!(&**lhs, Expr::Old { .. }),
        "old(...) should not be a call, got {lhs:?}"
    );

    let Expr::Unchanged { effect, .. } = &function.contract.ensures[1].condition else {
        panic!("unchanged(...) should not be a call");
    };
    assert_eq!(effect.effect.name, "Ledger");
}

#[test]
fn with_tells_a_handler_literal_from_the_body_block() {
    let stmts = body_of(
        "with InMemory { alice: 100 },\n\
         \x20    NullAudit\n\
         {\n\
         \x20   let x = 1\n\
         }",
    );
    let Stmt::Expr(Expr::With { handlers, body, .. }) = &stmts[0] else {
        panic!("expected a with, got {:?}", stmts[0]);
    };
    assert_eq!(handlers.len(), 2);
    assert!(matches!(handlers[0], Expr::StructLit { .. }));
    assert!(matches!(handlers[1], Expr::Ident(_)));
    assert_eq!(body.stmts.len(), 1);
}

#[test]
fn with_tells_an_empty_handler_literal_from_the_body_block() {
    // A record is allowed to have no fields, so a literal is allowed to have
    // none either, and the rule that looks for `name:` cannot see one because
    // there is no name to look at. `{ } {` decides it: an empty block is the
    // value `()` and nothing in this language puts a block straight after a
    // value.
    let stmts = body_of(
        "with Quiet { }\n\
         {\n\
         \x20   let x = 1\n\
         }",
    );
    let Stmt::Expr(Expr::With { handlers, body, .. }) = &stmts[0] else {
        panic!("expected a with, got {:?}", stmts[0]);
    };
    assert_eq!(handlers.len(), 1);
    assert!(matches!(handlers[0], Expr::StructLit { .. }));
    assert_eq!(body.stmts.len(), 1);
}

#[test]
fn a_handler_with_no_state_still_needs_no_braces() {
    // The spelling everything in the repository uses, and the one that has to
    // keep working: a brace with statements behind it is the body.
    let stmts = body_of(
        "with Quiet {\n\
         \x20   let x = 1\n\
         }",
    );
    let Stmt::Expr(Expr::With { handlers, body, .. }) = &stmts[0] else {
        panic!("expected a with, got {:?}", stmts[0]);
    };
    assert_eq!(handlers.len(), 1);
    assert!(matches!(handlers[0], Expr::Ident(_)));
    assert_eq!(body.stmts.len(), 1);
}

#[test]
fn a_for_accumulator_can_start_as_an_empty_record() {
    let stmts = body_of("for n in ns with seen = Empty { } {\n  seen\n}");
    let Stmt::Expr(Expr::For {
        accumulator, body, ..
    }) = &stmts[0]
    else {
        panic!("expected a for, got {:?}", stmts[0]);
    };
    let accumulator = accumulator.as_ref().expect("an accumulator");
    assert!(
        matches!(&*accumulator.init, Expr::StructLit { .. }),
        "{:?}",
        accumulator.init
    );
    assert_eq!(body.stmts.len(), 0);
}

#[test]
fn a_trailing_expression_is_the_block_value() {
    let parsed = parse_ok("module a\n\nfn f() -> Int {\n  let x = 1\n  x + 1\n}\n");
    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    assert_eq!(function.body.stmts.len(), 1);
    assert!(function.body.tail.is_some());
}

#[test]
fn match_arms_cover_the_pattern_forms() {
    let stmts = body_of(
        "match result {\n\
         \x20 Ok(receipt) => a(receipt),\n\
         \x20 Err(InsufficientFunds { available }) => b(available),\n\
         \x20 Err(LimitExceeded) => c(),\n\
         \x20 _ => d(),\n\
         }",
    );
    let Stmt::Expr(Expr::Match { arms, .. }) = &stmts[0] else {
        panic!("expected a match, got {:?}", stmts[0]);
    };
    assert_eq!(arms.len(), 4);
    assert!(matches!(arms[0].pattern, Pattern::Tuple { .. }));
    let Pattern::Tuple { elements, .. } = &arms[1].pattern else {
        panic!("expected Err(...)");
    };
    assert!(matches!(elements[0], Pattern::Record { .. }));
    assert!(matches!(arms[3].pattern, Pattern::Wildcard(_)));
}

#[test]
fn an_arm_can_name_alternatives() {
    let stmts = body_of(
        "match token {\n\
         \x20 Plus | Times | Close => a(),\n\
         \x20 Open => b(),\n\
         }",
    );
    let Stmt::Expr(Expr::Match { arms, .. }) = &stmts[0] else {
        panic!("expected a match, got {:?}", stmts[0]);
    };
    let Pattern::OneOf { alternatives, .. } = &arms[0].pattern else {
        panic!("expected alternatives, got {:?}", arms[0].pattern);
    };
    assert_eq!(alternatives.len(), 3);
    // A single pattern is still a single pattern, so nothing that already
    // parsed gained a wrapper.
    assert!(matches!(arms[1].pattern, Pattern::Path { .. }));
}

#[test]
fn a_closure_after_an_arm_is_still_a_closure() {
    // The only `|` that can follow a pattern in an arm is an alternative, and
    // the only `|` that can start an expression is a closure. Worth pinning,
    // because they are the same character in adjacent positions.
    let stmts = body_of(
        "match token {\n\
         \x20 Open => apply(|n: Int| n + 1),\n\
         \x20 Close => b(),\n\
         }",
    );
    let Stmt::Expr(Expr::Match { arms, .. }) = &stmts[0] else {
        panic!("expected a match");
    };
    assert_eq!(arms.len(), 2);
    assert!(matches!(arms[0].pattern, Pattern::Path { .. }));
}

// -- spans -----------------------------------------------------------------

#[test]
fn node_spans_land_on_the_right_source_text() {
    let source = "module a\n\nfn twice(n: Int) -> Int { n + n }\n";
    let (_, parsed) = parse_source(source);
    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };

    assert_eq!(&source[function.sig.name.span.as_range()], "twice");
    assert_eq!(&source[function.sig.params[0].span.as_range()], "n: Int");
    assert_eq!(&source[function.body.span.as_range()], "{ n + n }");
    assert_eq!(
        &source[function.body.tail.as_ref().unwrap().span().as_range()],
        "n + n"
    );
}

// -- function types --------------------------------------------------------

#[test]
fn a_declaration_may_carry_type_parameters() {
    // Only in a declaration, where the `<` cannot be a comparison, which is
    // why this needs no lookahead and `f<a>(b)` in expression position is not
    // a thing the parser has to think about.
    let (_, parsed) =
        parse_source("module a\n\nfn apply<A, B>(f: Fn(A) -> B, value: A) -> B { f(value) }\n");
    assert!(
        parsed.diagnostics.is_empty(),
        "{:?}",
        codes_of(&parsed.diagnostics)
    );

    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    assert_eq!(function.sig.generics.len(), 2);
    assert_eq!(function.sig.generics[0].name, "A");
    assert_eq!(function.sig.generics[1].name, "B");
}

#[test]
fn a_declaration_without_them_has_none() {
    let (_, parsed) = parse_source("module a\n\nfn f(n: Int) -> Int { n }\n");
    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    assert!(function.sig.generics.is_empty());
}

#[test]
fn a_function_type_is_a_type() {
    let (_, parsed) = parse_source(
        "module a\n\nfn apply(f: Fn(Int, Int) -> Int, n: Int) -> Fn(Int) -> Int { f }\n",
    );
    assert!(
        parsed.diagnostics.is_empty(),
        "{:?}",
        codes_of(&parsed.diagnostics)
    );

    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    let Some(deed_ast::Type::Fn { params, .. }) = &function.sig.params[0].ty else {
        panic!("the first parameter should be a function type");
    };
    assert_eq!(params.len(), 2);
    assert!(matches!(function.sig.ret, Some(deed_ast::Type::Fn { .. })));
}

#[test]
fn a_function_type_may_carry_a_row() {
    let (_, parsed) = parse_source(
        "module a\n\nfn apply(f: Fn(Int) uses Log.note, Audit -> Int) -> Int { f(1) }\n",
    );
    assert!(
        parsed.diagnostics.is_empty(),
        "{:?}",
        codes_of(&parsed.diagnostics)
    );

    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    let Some(deed_ast::Type::Fn { row, .. }) = &function.sig.params[0].ty else {
        panic!("the parameter should be a function type");
    };
    assert_eq!(row.len(), 2);
    assert_eq!(row[0].effect.name, "Log");
    assert_eq!(row[0].operation.as_ref().unwrap().name, "note");
    assert_eq!(row[1].effect.name, "Audit");
    assert!(row[1].operation.is_none());
}

#[test]
fn a_row_on_a_returned_function_type_is_still_the_types_own() {
    // The row goes before the arrow precisely so this parses. After the return
    // type it would be the declaration's own contract, and there would be no
    // way to tell which one was meant.
    let (_, parsed) = parse_source(
        "module a\n\nfn make() -> Fn(Int) uses Log.note -> Int\n  uses Audit,\n{ f }\n",
    );
    assert!(
        parsed.diagnostics.is_empty(),
        "{:?}",
        codes_of(&parsed.diagnostics)
    );

    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    let Some(deed_ast::Type::Fn { row, .. }) = &function.sig.ret else {
        panic!("the return type should be a function type");
    };
    assert_eq!(row.len(), 1);
    assert_eq!(row[0].effect.name, "Log");
    assert_eq!(function.contract.uses.len(), 1);
    assert_eq!(function.contract.uses[0].effect.name, "Audit");
}

#[test]
fn a_function_type_needs_its_return_type() {
    // One way to write a thing. A function type with no arrow reads like an
    // unfinished one, so it is refused rather than defaulted to `()`.
    let (_, parsed) = parse_source("module a\n\nfn f(g: Fn(Int)) -> Int { 0 }\n");
    assert!(!parsed.diagnostics.is_empty());
}

// -- contract rules --------------------------------------------------------

#[test]
fn contract_clauses_must_be_in_canonical_order() {
    let (sources, parsed) =
        parse_source("module a\n\nfn f() -> Int\n  ensures ok => true,\n  where x > 0,\n{ 0 }\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::CONTRACT_CLAUSE_ORDER]
    );
    let rendered = render_human(&sources, &parsed.diagnostics[0]);
    assert!(
        rendered.contains("`where` must come before `ensures`"),
        "{rendered}"
    );
}

#[test]
fn a_repeated_clause_is_reported_and_still_collected() {
    let (_, parsed) = parse_source("module a\n\nfn f() -> Int\n  uses A,\n  uses B,\n{ 0 }\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::DUPLICATE_CONTRACT_CLAUSE]
    );
    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    // Reported, but not thrown away. Later passes still see both effects.
    assert_eq!(function.contract.uses.len(), 2);
}

#[test]
fn an_ensures_outcome_must_be_ok_or_err() {
    let (sources, parsed) =
        parse_source("module a\n\nfn f() -> Int\n  ensures maybe => true,\n{ 0 }\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::INVALID_ENSURES_OUTCOME]
    );
    assert!(render_human(&sources, &parsed.diagnostics[0]).contains("expected `ok` or `err`"));
}

// The three places `ok` stands, one test each. They are in `SOFT_KEYWORDS`
// because of the first one, and the other two are what that must not cost.

#[test]
fn an_ensures_outcome_is_read_by_name() {
    let parsed = parse_ok(
        "module a\n\nfn f() -> Int\n  ensures\n    ok => true,\n    err => true,\n{ 0 }\n",
    );
    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    let outcomes: Vec<Outcome> = function
        .contract
        .ensures
        .iter()
        .map(|e| e.outcome)
        .collect();
    assert_eq!(outcomes, [Outcome::Ok, Outcome::Err]);
}

#[test]
fn ok_and_err_are_still_names_when_they_are_called() {
    // Neither word is reserved, and the position they mean something in is
    // the `ensures` clause above. Everywhere else they are the two prelude
    // constructors, which is how nearly every call in the corpus reaches
    // them.
    let stmts = body_of("ok(1)");
    let Stmt::Expr(Expr::Call { callee, .. }) = &stmts[0] else {
        panic!("expected a call, got {:?}", stmts[0]);
    };
    assert!(matches!(&**callee, Expr::Ident(name) if name.name == "ok"));

    let stmts = body_of("err(\"no\")");
    let Stmt::Expr(Expr::Call { callee, .. }) = &stmts[0] else {
        panic!("expected a call, got {:?}", stmts[0]);
    };
    assert!(matches!(&**callee, Expr::Ident(name) if name.name == "err"));
}

#[test]
fn ok_and_err_still_head_a_pattern() {
    let stmts = body_of("match r {\n  ok(value) => a(value),\n  err(why) => b(why),\n}");
    let Stmt::Expr(Expr::Match { arms, .. }) = &stmts[0] else {
        panic!("expected a match, got {:?}", stmts[0]);
    };
    assert_eq!(arms.len(), 2);
    for arm in arms {
        let Pattern::Tuple { path, elements, .. } = &arm.pattern else {
            panic!("expected a payload pattern, got {:?}", arm.pattern);
        };
        let [head] = &path[..] else {
            panic!("expected one segment, got {path:?}");
        };
        assert!(head.name == "ok" || head.name == "err", "{}", head.name);
        assert_eq!(elements.len(), 1);
    }
}

#[test]
fn a_star_effect_reference_parses() {
    let parsed = parse_ok("module a\n\nfn main(sys: System) -> Int\n  uses sys.*,\n{ 0 }\n");
    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    assert!(function.contract.uses[0].all);
    assert_eq!(function.contract.uses[0].effect.name, "sys");
}

// -- recovery --------------------------------------------------------------

#[test]
fn a_missing_module_declaration_is_reported_once() {
    let (sources, parsed) = parse_source("fn f() -> Int { 0 }\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::MISSING_MODULE_DECLARATION]
    );
    // The rest of the file still parses.
    assert_eq!(parsed.module.items.len(), 1);
    assert!(render_human(&sources, &parsed.diagnostics[0]).contains("`module`"));
}

#[test]
fn a_broken_function_does_not_hide_the_next_one() {
    let (_, parsed) = parse_source(
        "module a\n\n\
         fn broken(x: ) -> Int { 0 }\n\
         \n\
         fn healthy(n: Int) -> Int { n }\n",
    );

    assert!(parsed.has_errors());
    let names: Vec<&str> = parsed
        .module
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Function(f) => Some(f.sig.name.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        names.contains(&"healthy"),
        "recovery lost the second function, got {names:?}"
    );
}

#[test]
fn several_unrelated_errors_are_all_reported() {
    let (_, parsed) = parse_source(
        "module a\n\n\
         fn one() -> Int { 0 }\n\
         \n\
         42\n\
         \n\
         fn two() -> Int\n\
         \x20 ensures nope => true,\n\
         { 0 }\n",
    );

    let codes = codes_of(&parsed.diagnostics);
    assert!(
        codes.contains(&codes::EXPECTED_DECLARATION),
        "expected a declaration error, got {codes:?}"
    );
    assert!(
        codes.contains(&codes::INVALID_ENSURES_OUTCOME),
        "the second error should survive the first, got {codes:?}"
    );
}

#[test]
fn an_unclosed_brace_reports_once_and_suggests_a_fix() {
    let (_, parsed) = parse_source("module a\n\nfn f() -> Int {\n  let x = 1\n");
    assert_eq!(codes_of(&parsed.diagnostics), vec![codes::UNEXPECTED_TOKEN]);
    let fix = parsed.diagnostics[0].fix.as_ref().unwrap();
    assert_eq!(fix.edits[0].replacement, "}");
}

#[test]
fn a_bad_expression_becomes_an_error_node_rather_than_a_hole() {
    let (_, parsed) = parse_source("module a\n\nfn f() -> Int {\n  let x = ,\n}\n");
    assert!(parsed.has_errors());
    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    let Stmt::Let { init, .. } = &function.body.stmts[0] else {
        panic!("expected a let, got {:?}", function.body.stmts[0]);
    };
    assert!(init.is_error());
}

#[test]
fn parsing_always_terminates_on_junk() {
    // Not about the diagnostics, only that nothing loops forever.
    for source in [
        "module",
        "fn",
        "fn f(",
        "module a\n\nfn f() -> { }",
        "module a\n\nrecord R {",
        "module a\n\nchoice C { A(",
        "}}}}",
        "module a\n\nfn f() { match { } }",
        "module a\n\nhandler H implements { }",
    ] {
        let mut sources = SourceMap::new();
        let file = sources.add("junk.deed", source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        assert!(parsed.has_errors(), "{source:?} should not parse cleanly");
    }
}

#[test]
fn an_empty_file_reports_only_the_missing_module() {
    let (_, parsed) = parse_source("");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::MISSING_MODULE_DECLARATION]
    );
    assert!(parsed.module.items.is_empty());
}

// -- a match with no commas ---------------------------------------------------
//
// Arms one to a line and nothing between them is what a reader arriving from a
// language whose arms end at the newline writes. Before this, the first arm
// swallowed the rest of the match and what came back was nine diagnostics for
// one comma, none of which said comma.

const ARMS: &str = "module a\n\nchoice Grade {\n    Low,\n    High,\n}\n\n\
                    fn describe(mark: Grade) -> String {\n    match mark {\n        \
                    Low => \"low\"\n        High => \"high\"\n    }\n}\n";

#[test]
fn a_match_arm_with_no_comma_says_so_and_says_where() {
    let (sources, parsed) = parse_source(ARMS);
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::MISSING_COMMA],
        "one comma, one diagnostic"
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("separated by commas"), "{text}");
}

/// The point of carrying on rather than stopping: the arms after the missing
/// comma are arms, so the match is whole and every later pass sees the program
/// that was meant.
#[test]
fn the_arms_after_the_missing_comma_are_still_arms() {
    let (_, parsed) = parse_source(ARMS);
    let mut counted = 0;
    for item in &parsed.module.items {
        if let deed_ast::Item::Function(function) = item {
            for statement in &function.body.stmts {
                if let deed_ast::Stmt::Expr(deed_ast::Expr::Match { arms, .. }) = statement {
                    counted = arms.len();
                }
            }
            if let Some(deed_ast::Expr::Match { arms, .. }) = function.body.tail.as_deref() {
                counted = arms.len();
            }
        }
    }
    assert_eq!(counted, 2, "both arms should have been read");
}

/// Three arms with no commas is two commas missing, not one and then rubble.
#[test]
fn every_missing_comma_is_reported() {
    let (_, parsed) = parse_source(
        "module a\n\nchoice Grade {\n    Low,\n    Mid,\n    High,\n}\n\n\
         fn describe(mark: Grade) -> String {\n    match mark {\n        \
         Low => \"low\"\n        Mid => \"mid\"\n        High => \"high\"\n    }\n}\n",
    );
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::MISSING_COMMA, codes::MISSING_COMMA]
    );
}

/// The last arm is allowed to go without one, which is what the corpus writes
/// and what the formatter prints.
#[test]
fn the_last_arm_needs_no_comma() {
    let (_, parsed) = parse_source(
        "module a\n\nchoice Grade {\n    Low,\n    High,\n}\n\n\
         fn describe(mark: Grade) -> String {\n    match mark {\n        \
         Low => \"low\",\n        High => \"high\"\n    }\n}\n",
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
}

/// A `choice` written the same way used to say "insert `}`", which is an
/// answer to a question nobody asked. It is the first thing a model wrote on
/// the grade task.
#[test]
fn a_choice_variant_with_no_comma_says_so() {
    let (sources, parsed) =
        parse_source("module a\n\nchoice Grade {\n    Low\n    Mid\n    High,\n}\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::MISSING_COMMA, codes::MISSING_COMMA]
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("choice variants are separated"), "{text}");
}

/// And the variants after it are still variants, which is what stops the rest
/// of the file being about a choice with one case in it.
#[test]
fn the_variants_after_the_missing_comma_are_still_variants() {
    let (_, parsed) = parse_source("module a\n\nchoice Grade {\n    Low\n    Mid\n    High,\n}\n");
    let counted = parsed
        .module
        .items
        .iter()
        .find_map(|item| match item {
            deed_ast::Item::Choice(choice) => Some(choice.variants.len()),
            _ => None,
        })
        .expect("the choice should have parsed");
    assert_eq!(counted, 3);
}

#[test]
fn a_record_field_with_no_comma_says_so() {
    let (sources, parsed) =
        parse_source("module a\n\nrecord Point {\n    x: Int\n    y: Int,\n}\n");
    assert_eq!(codes_of(&parsed.diagnostics), vec![codes::MISSING_COMMA]);

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("record fields are separated"), "{text}");
}

/// A variant with fields ends at its closing brace, so that is where the comma
/// goes. Measuring to the name would put it in the middle of the variant.
#[test]
fn the_comma_after_a_variant_with_fields_goes_after_the_fields() {
    let (sources, parsed) = parse_source(
        "module a\n\nchoice Shape {\n    Circle { radius: Int }\n    Square { side: Int },\n}\n",
    );
    assert_eq!(codes_of(&parsed.diagnostics), vec![codes::MISSING_COMMA]);
    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("Circle { radius: Int }"), "{text}");
}

/// The last one goes without, in both, and the closing brace on its own line
/// is not the next item.
///
/// The half that says this is a repair rather than a new rule: every
/// declaration in this repository is written without the trailing comma
/// somewhere, and a check that asked for one would be asking the corpus to
/// change.
#[test]
fn the_last_item_of_a_declaration_needs_no_comma() {
    let (_, parsed) = parse_source(
        "module a\n\nchoice Grade {\n    Low,\n    High\n}\n\n\
         record Point {\n    x: Int,\n    y: Int\n}\n",
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
}

// -- the other arrow -------------------------------------------------------
//
// `->` where `=>` belongs. Both arrows are in the language and they are one
// key apart, and the arm used to end at the pattern, so the body after it was
// read as a statement and one slip cost four diagnostics.

const THIN: &str = "module a\n\nchoice Grade {\n    Low,\n    High,\n}\n\n\
                    fn describe(mark: Grade) -> String {\n    match mark {\n        \
                    Low -> \"low\",\n        High -> \"high\",\n    }\n}\n";

#[test]
fn a_match_arm_written_with_the_thin_arrow_says_which_arrow_it_wants() {
    let (sources, parsed) = parse_source(THIN);
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::WRONG_ARROW, codes::WRONG_ARROW],
        "one per arm, and nothing else"
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("written with `=>`"), "{text}");
    assert!(text.contains("`->` is the one in a signature"), "{text}");
}

/// Stepped over rather than stopped at, so the body is still the arm's body.
#[test]
fn the_arms_written_with_the_thin_arrow_are_still_arms() {
    let (_, parsed) = parse_source(THIN);
    let mut counted = 0;
    for item in &parsed.module.items {
        if let deed_ast::Item::Function(function) = item
            && let Some(deed_ast::Expr::Match { arms, .. }) = function.body.tail.as_deref()
        {
            counted = arms.len();
        }
    }
    assert_eq!(counted, 2, "both arms should have been read");
}

/// And it is offered, so `deed fix` puts it right without the author reading
/// anything.
#[test]
fn the_thin_arrow_comes_with_the_right_one() {
    let (_, parsed) = parse_source(THIN);
    let fix = parsed.diagnostics[0]
        .fix
        .as_ref()
        .expect("the arrow should come with a fix");
    assert_eq!(fix.edits[0].replacement, "=>");
}

// -- an obligation with no outcome -----------------------------------------

#[test]
fn an_ensures_with_no_outcome_says_the_outcome_is_missing() {
    let (sources, parsed) =
        parse_source("module a\n\nfn f(n: Int) -> Int\n  ensures\n    result == n,\n{ n }\n");
    assert_eq!(
        codes_of(&parsed.diagnostics),
        vec![codes::INVALID_ENSURES_OUTCOME],
        "one gap, one diagnostic: the missing `=>` is the same mistake"
    );

    let text = render_human(&sources, &parsed.diagnostics[0]);
    assert!(text.contains("does not say which outcome"), "{text}");
    let fix = parsed.diagnostics[0]
        .fix
        .as_ref()
        .expect("the outcome should come with a fix");
    assert_eq!(fix.edits[0].replacement, "ok => ");
}

/// What stood there was the condition, and reading it as one is the recovery.
#[test]
fn the_condition_of_an_outcomeless_ensures_is_still_read() {
    let (_, parsed) =
        parse_source("module a\n\nfn f(n: Int) -> Int\n  ensures\n    result == n,\n{ n }\n");
    let Item::Function(function) = &parsed.module.items[0] else {
        panic!("expected a function");
    };
    assert_eq!(function.contract.ensures.len(), 1);
    assert_eq!(function.contract.ensures[0].outcome, Outcome::Ok);
    assert!(matches!(
        function.contract.ensures[0].condition,
        Expr::Binary { .. }
    ));
}
