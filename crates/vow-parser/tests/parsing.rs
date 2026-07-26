//! Parser behaviour.
//!
//! The worked example covers the happy path. Everything else here is a way the
//! parser could quietly do the wrong thing, with recovery getting most of the
//! attention because that is what decides whether one mistake costs one round
//! trip or four.

use vow_ast::{BinaryOp, Expr, Item, Outcome, Pattern, Stmt};
use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_lexer::tokenize;
use vow_parser::{Parsed, codes, parse};

fn parse_source(src: &str) -> (SourceMap, Parsed) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.vow", src);
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
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
    let source = std::fs::read_to_string(path).expect("examples/transfer.vow should exist");

    let mut sources = SourceMap::new();
    let file = sources.add("examples/transfer.vow", source);
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
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
    let source = std::fs::read_to_string(path).unwrap();
    let mut sources = SourceMap::new();
    let file = sources.add("transfer.vow", source);
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

#[test]
fn a_closure_with_typed_parameters_parses() {
    parse_ok(
        "module a\n\nfn f(a: Int, b: Int) -> Int {\n    let add = |x: Int, y: Int| { x + y }\n    add(a, b)\n}\n",
    );
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
    let Some(vow_ast::Type::Fn { params, .. }) = &function.sig.params[0].ty else {
        panic!("the first parameter should be a function type");
    };
    assert_eq!(params.len(), 2);
    assert!(matches!(function.sig.ret, Some(vow_ast::Type::Fn { .. })));
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
    let Some(vow_ast::Type::Fn { row, .. }) = &function.sig.params[0].ty else {
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
    let Some(vow_ast::Type::Fn { row, .. }) = &function.sig.ret else {
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
        let file = sources.add("junk.vow", source);
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
