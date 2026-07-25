//! Type checking behaviour.
//!
//! Two things get most of the attention. What the checker refuses to say when
//! it does not know, since a false positive is worse than a missing check while
//! most of the language still comes from modules that cannot be loaded. And
//! refinements, since that is the first sliver of the Proven tier and the place
//! this language is eventually supposed to be interesting.

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_lexer::tokenize;
use vow_parser::parse;
use vow_resolve::resolve;
use vow_typeck::{Checked, Tier, Ty, Types, check, codes};

fn check_source(src: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.vow", src);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module);
    assert!(
        !resolved.has_errors(),
        "test source should resolve cleanly: {:?}",
        resolved
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );

    let checked = check(file, &parsed.module, &resolved.resolutions);
    (sources, checked)
}

fn check_ok(src: &str) -> Types {
    let (sources, checked) = check_source(src);
    if !checked.diagnostics.is_empty() {
        let rendered: Vec<String> = checked
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect();
        panic!("expected a clean check:\n{}", rendered.join("\n"));
    }
    checked.types
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

// -- the worked example ----------------------------------------------------

#[test]
fn the_worked_example_type_checks() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
    let source = std::fs::read_to_string(path).expect("examples/transfer.vow should exist");

    let mut sources = SourceMap::new();
    let file = sources.add("examples/transfer.vow", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module);
    assert!(!resolved.has_errors());

    let checked = check(file, &parsed.module, &resolved.resolutions);
    if !checked.diagnostics.is_empty() {
        panic!(
            "the worked example should type check:\n{}",
            rendered(&sources, &checked.diagnostics)
        );
    }
}

#[test]
fn the_worked_example_proves_its_refinements() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
    let source = std::fs::read_to_string(path).unwrap();

    let mut sources = SourceMap::new();
    let file = sources.add("transfer.vow", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module);
    let checked = check(file, &parsed.module, &resolved.resolutions);

    assert!(
        checked.types.obligations_at(Tier::Proven) > 0,
        "the example should discharge something statically"
    );
    assert_eq!(
        checked.types.obligations_at(Tier::Guarded),
        0,
        "nothing in the example should have needed a runtime check"
    );
}

// -- what the checker refuses to guess -------------------------------------

#[test]
fn a_name_from_an_unloaded_module_types_as_unknown() {
    // Almost everything interesting still comes from modules the compiler
    // cannot see, so this has to be silent rather than wrong.
    check_ok(
        "module a\n\n\
         use other.{Thing, make}\n\n\
         record R { field: Thing }\n\n\
         fn f() -> Thing { make(1, 2, 3) }\n\n\
         fn g(r: R) -> Int { r.field }\n",
    );
}

#[test]
fn an_operator_with_one_unknown_side_says_nothing() {
    check_ok(
        "module a\n\n\
         use other.{balance}\n\n\
         record Money { units: Int }\n\n\
         fn f(m: Money) -> Bool { balance() < m }\n\n\
         fn g(m: Money) -> Bool { balance() - m == balance() }\n",
    );
}

#[test]
fn an_operator_with_two_known_sides_is_checked() {
    let (sources, checked) = check_source(
        "module a\n\nrecord Money { units: Int }\n\nfn f(m: Money, n: Money) -> Int { m + n }\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&codes::TYPE_MISMATCH),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn the_question_mark_is_not_checked_yet() {
    // `Result` lives in another module, so there is nothing to check against.
    check_ok("module a\n\nuse std/result.{Result}\n\nfn f(r: Result) -> Int { r? }\n");
}

// -- refinements -----------------------------------------------------------

#[test]
fn a_literal_satisfying_a_refinement_is_proven() {
    let types = check_ok(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn take(n: Positive) -> Int { 0 }\n\n\
         fn f() -> Int { take(40) }\n",
    );
    assert_eq!(types.obligations_at(Tier::Proven), 1);
    assert_eq!(types.obligations_at(Tier::Guarded), 0);
}

#[test]
fn a_literal_violating_a_refinement_is_an_error() {
    let (sources, checked) = check_source(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn take(n: Positive) -> Int { 0 }\n\n\
         fn f() -> Int { take(0) }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::VIOLATED_REFINEMENT]
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("does not satisfy `Positive`"), "{text}");
    assert!(text.contains("the predicate it has to satisfy"), "{text}");
}

#[test]
fn an_unprovable_refinement_says_so_out_loud() {
    // The design promises Proven, Tested or Guarded, and that a contract never
    // quietly degrades into a runtime check. This is that promise being kept.
    let (sources, checked) = check_source(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn take(n: Positive) -> Int { 0 }\n\n\
         fn f(n: Int) -> Int { take(n) }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::UNPROVEN_REFINEMENT]
    );
    assert!(!checked.has_errors(), "it is a warning, not a rejection");
    assert_eq!(checked.types.obligations_at(Tier::Guarded), 1);

    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("becomes a runtime check"), "{text}");
    assert!(text.contains("Guarded"), "{text}");
}

#[test]
fn a_refinement_widens_to_its_base_without_an_obligation() {
    let types = check_ok(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn f(n: Positive) -> Int { n + 1 }\n",
    );
    assert!(types.obligations().is_empty());
}

#[test]
fn an_alias_without_a_predicate_is_transparent() {
    // It adds no information, so it should not add a distinction either.
    check_ok("module a\n\ntype Count = Int\n\nfn f(n: Count) -> Int { n }\n");
}

#[test]
fn a_self_referential_alias_is_reported_once() {
    let (_, checked) = check_source("module a\n\ntype Loop = Loop\n\nfn f(x: Loop) -> Int { 0 }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::TYPE_ALIAS_CYCLE]
    );
}

// -- records and choices ---------------------------------------------------

#[test]
fn a_literal_missing_fields_lists_them() {
    let (sources, checked) = check_source(
        "module a\n\nrecord R { a: Int, b: Int, c: Int }\n\nfn f() -> R { R { a: 1 } }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::MISSING_FIELDS]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("missing `b` and `c`"), "{text}");
}

#[test]
fn a_literal_with_an_unknown_field_lists_the_real_ones() {
    let (sources, checked) =
        check_source("module a\n\nrecord R { a: Int }\n\nfn f() -> R { R { a: 1, z: 2 } }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::UNKNOWN_FIELD]);
    assert!(rendered(&sources, &checked.diagnostics).contains("it has `a`"));
}

#[test]
fn the_shorthand_form_is_type_checked() {
    let (sources, checked) =
        check_source("module a\n\nrecord R { a: Int }\n\nfn f(a: Bool) -> R { R { a } }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    assert!(rendered(&sources, &checked.diagnostics).contains("the field it is assigned to"));
}

#[test]
fn field_access_on_a_missing_field_lists_what_is_there() {
    let (sources, checked) =
        check_source("module a\n\nrecord R { alpha: Int }\n\nfn f(r: R) -> Int { r.beta }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NO_SUCH_FIELD]);
    assert!(rendered(&sources, &checked.diagnostics).contains("it has `alpha`"));
}

#[test]
fn a_variant_literal_has_the_type_of_its_choice() {
    let types = check_ok(
        "module a\n\nchoice E { Full { n: Int }, Bare }\n\nfn f() -> E { Full { n: 1 } }\n",
    );
    let _ = types;
}

#[test]
fn a_variant_literal_checks_its_fields() {
    let (_, checked) = check_source(
        "module a\n\nchoice E { Full { n: Int } }\n\nfn f() -> E { Full { n: true } }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

// -- matches ---------------------------------------------------------------

#[test]
fn a_match_missing_variants_names_them() {
    let (sources, checked) = check_source(
        "module a\n\n\
         choice E { A, B, C }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A => 1,\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::NON_EXHAUSTIVE_MATCH]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("cover `B` and `C`"));
}

#[test]
fn a_wildcard_arm_on_a_choice_is_rejected() {
    let (sources, checked) = check_source(
        "module a\n\nchoice E { A, B, C }\n\nfn f(e: E) -> Int {\n  match e {\n    A => 1,\n    _ => 2,\n  }\n}\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::CATCH_ALL_ON_CHOICE]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("matches every variant of `E`"));
}

#[test]
fn a_bare_binding_arm_on_a_choice_is_rejected_too() {
    let (_, checked) = check_source(
        "module a\n\nchoice E { A, B }\n\nfn f(e: E) -> Int {\n  match e {\n    A => 1,\n    other => 2,\n  }\n}\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::CATCH_ALL_ON_CHOICE]
    );
}

#[test]
fn a_wildcard_is_fine_where_the_cases_cannot_be_listed() {
    // The rule is about choices. An `Int` has no variants to enumerate.
    check_ok("module a\n\nfn f(n: Int) -> Int {\n  match n {\n    0 => 1,\n    _ => 2,\n  }\n}\n");
}

#[test]
fn all_arms_must_agree() {
    let (_, checked) = check_source(
        "module a\n\nchoice E { A, B }\n\nfn f(e: E) -> Int {\n  match e {\n    A => 1,\n    B => true,\n  }\n}\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn a_variant_pattern_binds_its_fields_with_types() {
    check_ok(
        "module a\n\n\
         choice E { Full { n: Int }, Bare }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   Full { n } => n + 1,\n\
         \x20   Bare => 0,\n\
         \x20 }\n\
         }\n",
    );
}

// -- calls and returns -----------------------------------------------------

#[test]
fn a_wrong_argument_points_at_the_parameter() {
    let (sources, checked) =
        check_source("module a\n\nfn take(n: Int) -> Int { n }\n\nfn f() -> Int { take(true) }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("expected `Int`, found `Bool`"), "{text}");
    assert!(text.contains("the parameter it is passed to"), "{text}");
}

#[test]
fn the_wrong_number_of_arguments_points_at_the_declaration() {
    let (sources, checked) = check_source(
        "module a\n\nfn take(n: Int, m: Int) -> Int { n }\n\nfn f() -> Int { take(1) }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::WRONG_ARITY]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("takes 2 arguments, but 1 was given"),
        "{text}"
    );
    assert!(text.contains("declared here"), "{text}");
}

#[test]
fn calling_something_that_is_not_a_function_is_reported() {
    let (_, checked) = check_source("module a\n\nfn f(n: Int) -> Int { n(1) }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_CALLABLE]);
}

#[test]
fn a_body_must_produce_the_declared_return_type() {
    let (sources, checked) = check_source("module a\n\nfn f() -> Int { true }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    assert!(rendered(&sources, &checked.diagnostics).contains("the declared return type"));
}

#[test]
fn a_body_ending_in_return_is_accepted() {
    check_ok("module a\n\nfn f(n: Int) -> Int {\n  return n\n}\n");
}

#[test]
fn a_return_is_checked_against_the_signature() {
    let (_, checked) = check_source("module a\n\nfn f() -> Int {\n  return true\n}\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn an_if_without_an_else_must_produce_nothing() {
    check_ok("module a\n\nfn f(b: Bool, n: Int) -> Int {\n  if b {\n    return n\n  }\n  0\n}\n");
}

#[test]
fn both_branches_of_an_if_must_agree() {
    let (_, checked) =
        check_source("module a\n\nfn f(b: Bool) -> Int {\n  if b { 1 } else { true }\n}\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

// -- contracts -------------------------------------------------------------

#[test]
fn contract_clauses_must_be_conditions() {
    let (_, checked) = check_source("module a\n\nfn f(n: Int) -> Int\n  where n,\n{ n }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn a_contract_may_talk_about_the_parameters() {
    check_ok(
        "module a\n\n\
         fn f(n: Int, m: Int) -> Int\n\
         \x20 where n > 0, m > 0,\n\
         \x20 ensures ok => old(n) == n,\n\
         { n + m }\n",
    );
}

#[test]
fn unchanged_is_a_condition() {
    check_ok(
        "module a\n\n\
         effect Ledger {\n  fn post(n: Int) -> Int\n}\n\n\
         fn f() -> Int\n\
         \x20 uses Ledger.post,\n\
         \x20 ensures err => unchanged(Ledger),\n\
         { 0 }\n",
    );
}

// -- generics and robustness -----------------------------------------------

#[test]
fn a_local_type_takes_no_type_arguments() {
    let (sources, checked) =
        check_source("module a\n\nrecord R { n: Int }\n\nfn f(x: R<Int>) -> Int { 0 }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_GENERIC]);
    assert!(rendered(&sources, &checked.diagnostics).contains("does not take type arguments"));
}

#[test]
fn an_imported_type_may_take_type_arguments() {
    check_ok("module a\n\nuse std/result.{Result}\n\nfn f() -> Result<Int, Int> { 0 }\n");
}

#[test]
fn a_value_used_as_a_type_is_reported() {
    let (_, checked) =
        check_source("module a\n\nfn thing() -> Int { 0 }\n\nfn f(x: thing) -> Int { 0 }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_A_TYPE]);
}

#[test]
fn broken_input_does_not_panic_and_does_not_pile_on() {
    let mut sources = SourceMap::new();
    let file = sources.add(
        "broken.vow",
        "module a\n\nfn good() -> Int { 0 }\n\nfn bad(x: ) -> Int { , }\n\nfn worse() -> Nope { missing() }\n",
    );
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module);
    let checked = check(file, &parsed.module, &resolved.resolutions);

    // Names that failed to resolve become Unknown, and Unknown agrees with
    // everything, so the type checker adds nothing to a mess it did not make.
    assert!(
        checked.diagnostics.is_empty(),
        "type checker piled on after earlier errors:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn expression_types_are_recorded() {
    let src = "module a\n\nfn f() -> Int { 1 + 2 }\n";
    let types = check_ok(src);
    let start = src.find("1 + 2").unwrap() as u32;
    let span = vow_diagnostics::Span::new(start, start + 5);
    assert_eq!(types.type_of(span), Some(&Ty::Int));
}
