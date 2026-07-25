//! Effect checking behaviour.
//!
//! The "too wide" rule gets as much attention as "too narrow", because it is
//! the one that decides whether an effect row means anything.

use vow_ast::Item;
use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_effects::{Analysis, EffectItem, analyse, codes};
use vow_lexer::tokenize;
use vow_parser::parse;
use vow_resolve::{Universe, resolve};

fn analyse_source(
    src: &str,
) -> (
    SourceMap,
    vow_ast::Module,
    vow_resolve::Resolutions,
    Analysis,
) {
    analyse_source_in(src, &Universe::new())
}

fn analyse_source_in(
    src: &str,
    universe: &Universe,
) -> (
    SourceMap,
    vow_ast::Module,
    vow_resolve::Resolutions,
    Analysis,
) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.vow", src);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module, universe);
    assert!(!resolved.has_errors(), "test source should resolve cleanly");

    let analysis = analyse(file, &parsed.module, &resolved.resolutions);
    (sources, parsed.module, resolved.resolutions, analysis)
}

/// A universe holding each of `modules`, parsed from source.
///
/// An import with nothing behind it is an error now, so a test about an
/// imported effect needs the effect and every operation it names to exist on
/// the other side.
fn universe_of(modules: &[&str]) -> Universe {
    let mut universe = Universe::new();
    let mut sources = SourceMap::new();
    for (index, source) in modules.iter().enumerate() {
        let file = sources.add(format!("dep{index}.vow"), *source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        universe.add(&parsed.module);
    }
    universe
}

/// A module declaring the `Ledger` effect the import tests reach for.
const LEDGER_MODULE: &str = "module ledger\n\n\
     effect Ledger {\n\
     \x20 fn balance(id: Int) -> Int\n\
     \x20 fn post(amount: Int) -> ()\n\
     }\n";

fn analyse_ok(src: &str) {
    let (sources, _, _, analysis) = analyse_source(src);
    if !analysis.diagnostics.is_empty() {
        panic!(
            "expected a clean analysis:\n{}",
            rendered(&sources, &analysis.diagnostics)
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

/// A module with a `Ledger` effect, so the tests can get to the point.
fn with_ledger(body: &str) -> String {
    format!(
        "module a\n\n\
         effect Ledger {{\n\
         \x20 fn balance(id: Int) -> Int\n\
         \x20 fn post(amount: Int) -> ()\n\
         }}\n\n\
         effect Audit {{\n\
         \x20 fn append(note: Int) -> ()\n\
         }}\n\n\
         {body}"
    )
}

// -- the worked example ----------------------------------------------------

#[test]
fn the_worked_example_passes_effect_checking() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
    let source = std::fs::read_to_string(path).expect("examples/transfer.vow should exist");

    let mut sources = SourceMap::new();
    let file = sources.add("examples/transfer.vow", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());
    assert!(!resolved.has_errors());

    let analysis = analyse(file, &parsed.module, &resolved.resolutions);
    if !analysis.diagnostics.is_empty() {
        panic!(
            "the worked example should pass effect checking:\n{}",
            rendered(&sources, &analysis.diagnostics)
        );
    }
}

#[test]
fn the_worked_example_row_is_exactly_what_the_body_does() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
    let source = std::fs::read_to_string(path).unwrap();

    let mut sources = SourceMap::new();
    let file = sources.add("transfer.vow", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());
    let analysis = analyse(file, &parsed.module, &resolved.resolutions);

    let transfer = parsed
        .module
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(f) if f.sig.name.name == "transfer" => Some(f),
            _ => None,
        })
        .unwrap();
    let def = resolved
        .resolutions
        .resolution(transfer.sig.name.span)
        .unwrap();

    let declared = analysis.effects.declared(def).unwrap();
    let performed = analysis.effects.performed(def).unwrap();
    assert_eq!(declared.len(), 3);
    assert_eq!(
        declared, performed,
        "the row should be tight, not merely sufficient"
    );

    // `Ledger.total` is called in the `ensures` clause and is deliberately not
    // in the row, because specification is not action.
    let total = performed
        .iter()
        .any(|item| item.operation.as_deref() == Some("total"));
    assert!(
        !total,
        "a contract should not have to pay for what it observes"
    );
}

/// The measurement #12 asked for.
///
/// `03-effects.md` admits that effect systems keep dying on annotation burden.
/// This is that worry as a number rather than a paragraph. The bounds are loose
/// on purpose: the point is to notice growth, not to freeze today's figures.
#[test]
fn the_annotation_burden_is_measured() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
    let source = std::fs::read_to_string(path).unwrap();

    let mut sources = SourceMap::new();
    let file = sources.add("transfer.vow", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);

    let functions: Vec<_> = parsed
        .module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(f) => Some(f),
            _ => None,
        })
        .collect();

    let total = functions.len();
    let annotated = functions
        .iter()
        .filter(|f| !f.contract.uses.is_empty())
        .count();
    let longest = functions
        .iter()
        .map(|f| f.contract.uses.len())
        .max()
        .unwrap_or(0);

    assert!(total > 0);
    assert!(
        longest <= 5,
        "rows are getting long: the worst is {longest} entries, which is where effect systems start losing people"
    );
    // Purity should be the common case, not the exception. If this ever needs
    // relaxing, that is the finding, not the test being wrong.
    assert!(
        annotated * 2 <= total,
        "{annotated} of {total} functions carry an effect row, and the default is supposed to be purity"
    );
}

// -- too narrow ------------------------------------------------------------

#[test]
fn performing_an_undeclared_effect_is_an_error() {
    let (sources, _, _, analysis) =
        analyse_source(&with_ledger("fn f() -> Int { Ledger.balance(1) }\n"));
    assert_eq!(
        codes_of(&analysis.diagnostics),
        vec![codes::UNDECLARED_EFFECT]
    );
    let text = rendered(&sources, &analysis.diagnostics);
    assert!(
        text.contains("performs `Ledger.balance` without declaring it"),
        "{text}"
    );
}

#[test]
fn an_effect_reached_through_a_called_function_counts() {
    let (_, _, _, analysis) = analyse_source(&with_ledger(
        "fn inner() -> Int\n  uses Ledger.balance,\n{ Ledger.balance(1) }\n\n\
         fn outer() -> Int { inner() }\n",
    ));
    assert_eq!(
        codes_of(&analysis.diagnostics),
        vec![codes::UNDECLARED_EFFECT]
    );
}

#[test]
fn declaring_the_whole_effect_covers_its_operations() {
    analyse_ok(&with_ledger(
        "fn f() -> Int\n  uses Ledger,\n{\n  Ledger.post(1)\n  Ledger.balance(2)\n}\n",
    ));
}

#[test]
fn one_operation_does_not_grant_another() {
    let (sources, _, _, analysis) = analyse_source(&with_ledger(
        "fn f() -> Int\n  uses Ledger.balance,\n{\n  Ledger.post(1)\n  Ledger.balance(2)\n}\n",
    ));
    assert_eq!(
        codes_of(&analysis.diagnostics),
        vec![codes::UNDECLARED_EFFECT]
    );
    assert!(rendered(&sources, &analysis.diagnostics).contains("`Ledger.post`"));
}

// -- too wide --------------------------------------------------------------

#[test]
fn declaring_an_effect_that_is_never_performed_is_an_error() {
    let (sources, _, _, analysis) = analyse_source(&with_ledger(
        "fn f() -> Int\n  uses Ledger.balance, Audit.append,\n{ Ledger.balance(1) }\n",
    ));
    assert_eq!(codes_of(&analysis.diagnostics), vec![codes::UNUSED_EFFECT]);
    let text = rendered(&sources, &analysis.diagnostics);
    assert!(
        text.contains("`Audit.append` is declared but never performed"),
        "{text}"
    );
    assert!(text.contains("only worth reading if it is tight"), "{text}");
}

#[test]
fn a_row_must_be_tight_not_merely_sufficient() {
    // Declaring the whole effect while only using one operation is over-wide,
    // which is the case a laxer rule would let through.
    let (_, _, _, analysis) = analyse_source(&with_ledger(
        "fn f() -> Int\n  uses Ledger, Audit,\n{ Ledger.balance(1) }\n",
    ));
    assert_eq!(codes_of(&analysis.diagnostics), vec![codes::UNUSED_EFFECT]);
}

// -- purity ----------------------------------------------------------------

#[test]
fn a_function_with_no_uses_clause_is_pure() {
    analyse_ok("module a\n\nfn double(n: Int) -> Int { n + n }\n");
}

#[test]
fn purity_is_transitive_through_calls() {
    analyse_ok("module a\n\nfn one() -> Int { 1 }\n\nfn two() -> Int { one() + one() }\n");
}

// -- closures --------------------------------------------------------------

#[test]
fn a_closure_charges_the_function_that_wrote_it() {
    // This is the conservative rule, not the right one. The right one puts the
    // row in the closure's type and charges the call site. What makes the
    // conservative one sound is that a closure cannot leave the function that
    // wrote it, so there is nowhere else the charge could land.
    let (sources, _, _, analysis) = analyse_source(&with_ledger(
        "fn f() -> Int {\n    let write = || { Ledger.post(1) }\n    write()\n    0\n}\n",
    ));
    assert_eq!(
        codes_of(&analysis.diagnostics),
        vec![codes::UNDECLARED_EFFECT],
        "{}",
        rendered(&sources, &analysis.diagnostics)
    );
}

#[test]
fn declaring_what_the_closure_does_is_enough() {
    analyse_ok(&with_ledger(
        "fn f() -> Int\n  uses\n    Ledger.post,\n{\n    let write = || { Ledger.post(1) }\n    write()\n    0\n}\n",
    ));
}

#[test]
fn a_closure_that_is_never_called_still_charges_its_author() {
    // The over-approximation, stated out loud. Writing a closure is enough,
    // because deciding whether it is ever called is deciding whether a
    // function value escapes, and the point of this rule is not having to.
    let (sources, _, _, analysis) = analyse_source(&with_ledger(
        "fn f() -> Int {\n    let unused = || { Ledger.post(1) }\n    0\n}\n",
    ));
    assert_eq!(
        codes_of(&analysis.diagnostics),
        vec![codes::UNDECLARED_EFFECT],
        "{}",
        rendered(&sources, &analysis.diagnostics)
    );
}

#[test]
fn a_row_that_only_the_closure_uses_is_not_too_wide() {
    // The other rule has to agree, or declaring the effect the closure needs
    // would trade one error for another and there would be nothing to write.
    analyse_ok(&with_ledger(
        "fn f() -> Int\n  uses\n    Ledger.post,\n{\n    let write = || { Ledger.post(1) }\n    write()\n    0\n}\n",
    ));
}

// -- termination -----------------------------------------------------------

/// The codes reported for `src`, for the tests that only care about those.
fn codes_for(src: &str) -> Vec<String> {
    let (_, _, _, analysis) = analyse_source(src);
    analysis
        .diagnostics
        .iter()
        .map(|d| d.code.to_string())
        .collect()
}

#[test]
fn a_function_that_calls_itself_has_to_say_so() {
    // No termination proving happens anywhere, so `factorial` counts. That is
    // the honest reading of "a loop the compiler cannot show terminates".
    assert_eq!(
        codes_for(
            "module a\n\nfn factorial(n: Int) -> Int {\n    if n <= 1 {\n        1\n    } else {\n        n * factorial(n - 1)\n    }\n}\n"
        ),
        vec![codes::UNDECLARED_EFFECT]
    );
}

#[test]
fn saying_so_is_enough() {
    analyse_ok(
        "module a\n\nfn forever(n: Int) -> Int\n  uses\n    Diverge,\n{\n    forever(n + 1)\n}\n",
    );
}

#[test]
fn mutual_recursion_catches_both_halves() {
    // Neither function calls itself. Between them they can loop forever, and
    // a rule that only looked at direct recursion would miss it entirely.
    assert_eq!(
        codes_for(
            "module a\n\n\
             fn even(n: Int) -> Bool {\n    if n == 0 {\n        true\n    } else {\n        odd(n - 1)\n    }\n}\n\n\
             fn odd(n: Int) -> Bool {\n    if n == 0 {\n        false\n    } else {\n        even(n - 1)\n    }\n}\n"
        ),
        vec![codes::UNDECLARED_EFFECT, codes::UNDECLARED_EFFECT]
    );
}

#[test]
fn calling_something_that_may_not_return_may_not_return() {
    // Straight out of the existing propagation: a call contributes the
    // callee's declared row, and this is in that row like anything else.
    assert_eq!(
        codes_for(
            "module a\n\n\
             fn forever() -> Int\n  uses\n    Diverge,\n{\n    forever()\n}\n\n\
             fn caller() -> Int {\n    forever()\n}\n"
        ),
        vec![codes::UNDECLARED_EFFECT]
    );
}

#[test]
fn declaring_it_without_recursing_is_too_wide() {
    // The tightness rule has to apply here too, or `Diverge` becomes the entry
    // everyone adds pre-emptively and the row stops meaning anything.
    assert_eq!(
        codes_for("module a\n\nfn f() -> Int\n  uses\n    Diverge,\n{\n    1\n}\n"),
        vec![codes::UNUSED_EFFECT]
    );
}

#[test]
fn a_closure_that_calls_a_diverging_function_charges_its_author() {
    assert_eq!(
        codes_for(
            "module a\n\n\
             fn forever() -> Int\n  uses\n    Diverge,\n{\n    forever()\n}\n\n\
             fn caller() -> Int {\n    let go = || { forever() }\n    go()\n    0\n}\n"
        ),
        vec![codes::UNDECLARED_EFFECT]
    );
}

#[test]
fn a_test_that_runs_something_diverging_needs_no_handler() {
    // There is nothing to install. Asking for a handler would be asking for a
    // thing that cannot be written.
    analyse_ok(
        "module a\n\n\
         fn forever(n: Int) -> Int\n  uses\n    Diverge,\n{\n    forever(n + 1)\n}\n\n\
         test \"runs it on purpose\" {\n    assert forever(0) == 0\n}\n",
    );
}

#[test]
fn a_contract_mentioning_a_recursive_function_is_not_a_call() {
    // Specification is not action, the same way it is not for any other
    // effect. A `where` clause naming a recursive function does not run it.
    analyse_ok(
        "module a\n\n\
         fn size() -> Int { 1 }\n\n\
         fn f(n: Int) -> Int\n  where\n    n > size(),\n{\n    n\n}\n",
    );
}

// -- specification is not action -------------------------------------------

#[test]
fn a_contract_may_observe_an_effect_without_declaring_it() {
    // `ensures` describes state rather than changing it. Making an obligation
    // cost permissions would be making obligations expensive to write, which is
    // the opposite of what this language wants.
    analyse_ok(&with_ledger(
        "fn f() -> Int\n\
         \x20 ensures ok => Ledger.balance(1) == old(Ledger.balance(1)),\n\
         { 0 }\n",
    ));
}

#[test]
fn unchanged_costs_nothing() {
    analyse_ok(&with_ledger(
        "fn f() -> Int\n  ensures err => unchanged(Ledger),\n{ 0 }\n",
    ));
}

// -- handlers and tests ----------------------------------------------------

#[test]
fn a_with_block_discharges_the_effect_its_handler_implements() {
    analyse_ok(&with_ledger(
        "handler InMemory implements Ledger {\n\
         \x20 state holdings: Int\n\n\
         \x20 fn balance(id) -> Int { holdings }\n\
         \x20 fn post(amount) -> () { }\n\
         }\n\n\
         test \"reads a balance\" {\n\
         \x20 with InMemory { holdings: 100 } {\n\
         \x20   assert Ledger.balance(1) == 100\n\
         \x20 }\n\
         }\n",
    ));
}

#[test]
fn a_handler_only_discharges_its_own_effect() {
    let (sources, _, _, analysis) = analyse_source(&with_ledger(
        "handler InMemory implements Ledger {\n\
         \x20 state holdings: Int\n\n\
         \x20 fn balance(id) -> Int { holdings }\n\
         }\n\n\
         test \"writes an audit note\" {\n\
         \x20 with InMemory { holdings: 1 } {\n\
         \x20   Audit.append(1)\n\
         \x20 }\n\
         }\n",
    ));
    assert_eq!(
        codes_of(&analysis.diagnostics),
        vec![codes::UNHANDLED_EFFECT]
    );
    assert!(rendered(&sources, &analysis.diagnostics).contains("`Audit.append`"));
}

#[test]
fn a_test_performing_an_effect_with_no_with_block_is_an_error() {
    let (_, _, _, analysis) = analyse_source(&with_ledger(
        "test \"forgot the handler\" {\n  Audit.append(1)\n}\n",
    ));
    assert_eq!(
        codes_of(&analysis.diagnostics),
        vec![codes::UNHANDLED_EFFECT]
    );
}

// -- rows across a module boundary -----------------------------------------

#[test]
fn a_row_naming_an_imported_effect_is_checked_like_any_other() {
    // An effect's operations are part of its declaration, so a row naming one
    // from another module is as checkable as a local one. Declaring an
    // operation the body never performs is still too wide.
    let (sources, _, _, analysis) = analyse_source_in(
        "module a\n\nuse ledger.{Ledger}\n\nfn f() -> Int\n  uses Ledger.post,\n{ 0 }\n",
        &universe_of(&[LEDGER_MODULE]),
    );
    assert_eq!(codes_of(&analysis.diagnostics), vec![codes::UNUSED_EFFECT]);
    let text = rendered(&sources, &analysis.diagnostics);
    assert!(
        text.contains("`Ledger.post` is declared but never performed"),
        "{text}"
    );
}

#[test]
fn an_imported_effect_performed_without_being_declared_is_reported() {
    let (sources, _, _, analysis) = analyse_source_in(
        "module a\n\nuse ledger.{Ledger}\n\nfn f() -> Int { Ledger.balance(1) }\n",
        &universe_of(&[LEDGER_MODULE]),
    );
    assert_eq!(
        codes_of(&analysis.diagnostics),
        vec![codes::UNDECLARED_EFFECT]
    );
    let text = rendered(&sources, &analysis.diagnostics);
    assert!(text.contains("Ledger.balance"), "{text}");
}

#[test]
fn a_tight_row_over_an_imported_effect_is_accepted() {
    let (sources, _, _, analysis) = analyse_source_in(
        "module a\n\nuse ledger.{Ledger}\n\nfn f() -> Int\n  uses Ledger.balance,\n{ Ledger.balance(1) }\n",
        &universe_of(&[LEDGER_MODULE]),
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &analysis.diagnostics)
    );
}

// -- rows that cannot be checked -------------------------------------------

#[test]
fn granting_everything_a_capability_carries_is_reported() {
    // This is the hole design/04-capabilities.md already worried about. The
    // compiler saying it out loud is better than reporting a clean check it
    // never performed.
    let (sources, _, _, analysis) =
        analyse_source("module a\n\nfn main(sys: System) -> Int\n  uses sys.*,\n{ 0 }\n");
    assert_eq!(
        codes_of(&analysis.diagnostics),
        vec![codes::UNVERIFIABLE_ROW]
    );
    let text = rendered(&sources, &analysis.diagnostics);
    assert!(
        text.contains("grants everything that capability carries"),
        "{text}"
    );
    assert!(
        text.contains("granting everything is the same as promising nothing"),
        "{text}"
    );
}

#[test]
fn an_unverifiable_row_suppresses_both_checks() {
    // Otherwise every entry would be reported as unused, which is noise about
    // something the compiler already admitted it cannot see. `sys.*` is the
    // remaining way to get an unverifiable row, since an imported effect is
    // checked properly now.
    let (_, _, _, analysis) = analyse_source(
        "module a\n\neffect Ledger {\n  fn post(n: Int) -> ()\n}\n\nfn main(sys: System) -> Int\n  uses sys.*, Ledger.post,\n{ 0 }\n",
    );
    assert_eq!(
        codes_of(&analysis.diagnostics)
            .iter()
            .filter(|code| **code == codes::UNUSED_EFFECT)
            .count(),
        0
    );
}

#[test]
fn a_uses_entry_naming_something_that_is_not_an_effect_is_rejected() {
    let (sources, _, _, analysis) = analyse_source(
        "module a\n\nrecord Money { units: Int }\n\nfn f() -> Int\n  uses Money,\n{ 0 }\n",
    );
    assert_eq!(codes_of(&analysis.diagnostics), vec![codes::NOT_AN_EFFECT]);
    assert!(rendered(&sources, &analysis.diagnostics).contains("is a record, not an effect"));
}

// -- robustness ------------------------------------------------------------

#[test]
fn broken_input_does_not_panic() {
    let mut sources = SourceMap::new();
    let file = sources.add(
        "broken.vow",
        "module a\n\nfn bad(x: ) -> Int { , }\n\nfn worse() -> Int\n  uses Nope.thing,\n{ 0 }\n",
    );
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());
    let analysis = analyse(file, &parsed.module, &resolved.resolutions);

    // `Nope` never resolved, so there is nothing to say that has not already
    // been said by an earlier pass.
    assert!(
        !codes_of(&analysis.diagnostics).contains(&codes::NOT_AN_EFFECT),
        "{}",
        rendered(&sources, &analysis.diagnostics)
    );
}

#[test]
fn rows_are_reported_for_inspection() {
    let (_, module, resolutions, analysis) = analyse_source(&with_ledger(
        "fn f() -> Int\n  uses Ledger.balance,\n{ Ledger.balance(1) }\n",
    ));

    let function = module
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(f) => Some(f),
            _ => None,
        })
        .unwrap();
    let def = resolutions.resolution(function.sig.name.span).unwrap();
    let ledger = analysis
        .effects
        .declared(def)
        .unwrap()
        .iter()
        .next()
        .unwrap()
        .effect;

    assert!(
        analysis
            .effects
            .performed(def)
            .unwrap()
            .covers(&EffectItem::operation(ledger, "balance"))
    );
}
