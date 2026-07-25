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
use vow_resolve::{Universe, resolve};
use vow_typeck::{Checked, Tier, Ty, Types, World, check, codes, surface};

/// The other modules a test source can see.
#[derive(Default)]
struct Deps {
    universe: Universe,
    world: World,
}

fn check_source(src: &str) -> (SourceMap, Checked) {
    check_source_in(src, &Deps::default())
}

fn check_source_in(src: &str, deps: &Deps) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.vow", src);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module, &deps.universe);
    assert!(
        !resolved.has_errors(),
        "test source should resolve cleanly: {:?}",
        resolved
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );

    let checked = check(file, &parsed.module, &resolved.resolutions, &deps.world);
    (sources, checked)
}

/// The other modules, parsed and lowered.
///
/// An import with nothing behind it is an error now, and a name from elsewhere
/// has a type now, so a test about one needs a real module declaring it.
fn universe_of(modules: &[&str]) -> Deps {
    let mut deps = Deps::default();
    let mut sources = SourceMap::new();
    for (index, source) in modules.iter().enumerate() {
        let file = sources.add(format!("dep{index}.vow"), *source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        deps.universe.add(&parsed.module);
    }

    // A second pass, because a module's surface needs its own names resolved
    // and that needs every module already registered.
    let mut sources = SourceMap::new();
    for (index, source) in modules.iter().enumerate() {
        let file = sources.add(format!("dep{index}.vow"), *source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        let resolved = resolve(file, &parsed.module, &deps.universe);
        if let Some(name) = &parsed.module.name {
            deps.world.insert(
                name.to_string_path(),
                surface(&parsed.module, &resolved.resolutions),
            );
        }
    }

    deps
}

fn check_ok(src: &str) -> Types {
    check_ok_in(src, &Deps::default())
}

fn check_ok_in(src: &str, deps: &Deps) -> Types {
    let (sources, checked) = check_source_in(src, deps);
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
    let resolved = resolve(file, &parsed.module, &Universe::new());
    assert!(!resolved.has_errors());

    let checked = check(file, &parsed.module, &resolved.resolutions, &World::new());
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
    let resolved = resolve(file, &parsed.module, &Universe::new());
    let checked = check(file, &parsed.module, &resolved.resolutions, &World::new());

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
fn a_name_from_another_module_has_a_type() {
    // A name that came from an import used to have no type at all, so nothing
    // done with it was ever checked. It is now identified by the module it
    // came from together with the name it was declared under, which needs no
    // numbering shared between modules.
    let src = "module a\n\n\
               use other.{Thing}\n\n\
               record R { field: Thing }\n\n\
               fn g(r: R) -> Thing { r.field }\n";
    let types = check_ok_in(
        src,
        &universe_of(&["module other\n\nrecord Thing { n: Int }\n"]),
    );

    let start = src.find("r.field").unwrap() as u32;
    let span = vow_diagnostics::Span::new(start, start + "r.field".len() as u32);
    assert_eq!(
        types.type_of(span),
        Some(&Ty::External {
            module: "other".into(),
            name: "Thing".into(),
        })
    );
}

#[test]
fn an_operator_with_one_unknown_side_says_nothing() {
    // `balanse` does not resolve, so it has no type, and a type that agrees
    // with everything is what keeps one mistake from being reported twice.
    // The resolver has already said the name is missing.
    let mut sources = SourceMap::new();
    let file = sources.add(
        "test.vow",
        "module a\n\n\
         record Money { units: Int }\n\n\
         fn f(m: Money) -> Bool { balanse() < m }\n\n\
         fn g(m: Money) -> Bool { balanse() - m == balanse() }\n",
    );
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());
    let checked = check(file, &parsed.module, &resolved.resolutions, &World::new());
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
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
fn the_question_mark_rejects_a_type_that_is_only_named_like_a_result() {
    // `?` went unchecked here while the `Result` in scope came from an import
    // and so had no type. It has one now, and a record another module happens
    // to have called `Result` is not the one the language provides.
    let (sources, checked) = check_source_in(
        "module a\n\nuse std/result.{Result}\n\nfn f(r: Result) -> Int { r? }\n",
        &universe_of(&["module std/result\n\nrecord Result { n: Int }\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_A_RESULT]);
    assert!(rendered(&sources, &checked.diagnostics).contains("`?` needs a `Result`"));
}

// -- operators -------------------------------------------------------------

#[test]
fn plus_joins_two_strings() {
    check_ok("module a\n\nfn greet(name: String) -> String { \"hello, \" + name }\n");
}

#[test]
fn plus_will_not_join_a_number_to_a_string() {
    // There is no conversion between them, so guessing which one the author
    // meant would be guessing.
    let (sources, checked) =
        check_source("module a\n\nfn f(n: Int) -> String { \"count: \" + n }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    assert!(rendered(&sources, &checked.diagnostics).contains("joined with this"));
}

#[test]
fn plus_still_adds_two_numbers() {
    check_ok("module a\n\nfn f(n: Int) -> Int { n + 1 }\n");
}

#[test]
fn strings_are_ordered() {
    check_ok("module a\n\nfn f(a: String, b: String) -> Bool { a < b }\n");
}

#[test]
fn a_record_is_not_ordered() {
    // This used to pass here and fail at runtime, with a message blaming the
    // interpreter for not implementing something that has nothing to implement.
    let (sources, checked) = check_source(
        "module a\n\nrecord Point { x: Int }\n\nfn f(a: Point, b: Point) -> Bool { a < b }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_ORDERED]);
    assert!(rendered(&sources, &checked.diagnostics).contains("there is none on `Point`"));
}

#[test]
fn a_choice_is_not_ordered_either() {
    let (_, checked) = check_source(
        "module a\n\nchoice Tone { Plain, Loud }\n\nfn f(a: Tone, b: Tone) -> Bool { a >= b }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_ORDERED]);
}

#[test]
fn two_types_that_do_not_match_are_one_mistake_not_two() {
    // Saying the sides disagree and then that the type they do not share has
    // no ordering is two diagnostics for one edit.
    let (_, checked) = check_source(
        "module a\n\nrecord Point { x: Int }\n\nfn f(a: Point, b: Int) -> Bool { a < b }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn anything_can_still_be_compared_for_equality() {
    // Equality is structural and total. Wanting to know whether two records
    // are the same is reasonable; wanting to know which one is larger is not.
    check_ok(
        "module a\n\nrecord Point { x: Int }\n\nfn f(a: Point, b: Point) -> Bool { a == b }\n",
    );
}

#[test]
fn a_refinement_over_a_string_is_ordered_through_its_base() {
    check_ok(
        "module a\n\n\
         type Name = String where length(value) > 0\n\n\
         fn f(a: Name, b: Name) -> Bool { a < b }\n",
    );
}

#[test]
fn length_takes_a_string_and_gives_a_number() {
    check_ok("module a\n\nfn f(s: String) -> Int { length(s) }\n");
}

#[test]
fn length_will_not_measure_a_number() {
    let (_, checked) = check_source("module a\n\nfn f(n: Int) -> Int { length(n) }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn a_length_is_never_negative() {
    // The prelude says so, so a refinement that only needs that much is
    // Proven rather than checked again at runtime.
    let types = check_ok(
        "module a\n\n\
         type Counted = Int where value >= 0\n\n\
         fn f(s: String) -> Counted { length(s) }\n",
    );
    assert_eq!(types.obligations_at(Tier::Proven), 1);
}

// -- handlers ---------------------------------------------------------------

const COUNTER: &str = "module a\n\n\
     effect Counter {\n    fn set(to: Int) -> ()\n    fn value() -> Int\n}\n\n";

#[test]
fn a_handler_operation_takes_its_types_from_the_effect() {
    // Nothing here writes `Int` anywhere, and `to + 1` still has to be
    // arithmetic. Before this, every parameter in every handler body was the
    // unknown type, so nothing done with one was checked at all.
    check_ok(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to) -> () {{\n    count = to + 1\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count\n  }}\n\
         }}\n"
    ));
}

#[test]
fn a_handler_operation_is_checked_against_those_types() {
    let (sources, checked) = check_source(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: String\n\n\
         \x20 fn set(to) -> () {{\n    count = to\n  }}\n\n\
         \x20 fn value() -> Int {{\n    1\n  }}\n\
         }}\n"
    ));
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    assert!(rendered(&sources, &checked.diagnostics).contains("found `Int`"));
}

#[test]
fn a_refined_state_field_is_an_obligation_like_any_other() {
    // This is the one that could not be written before. The parameter had no
    // type, unknown absorbs, and assigning it to a refined state field raised
    // nothing at all: no warning, no tier, no runtime check.
    let (_, checked) = check_source(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         effect Counter {\n    fn set(to: Int) -> ()\n}\n\n\
         handler InMemory implements Counter {\n\
         \x20 state count: Positive\n\n\
         \x20 fn set(to) -> () {\n    count = to\n  }\n\
         }\n",
    );
    assert!(!checked.has_errors());
    assert_eq!(checked.types.obligations_at(Tier::Guarded), 1);
}

#[test]
fn a_handler_operation_the_effect_does_not_have_is_an_error() {
    let (sources, checked) = check_source(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to) -> () {{\n    count = to\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count\n  }}\n\n\
         \x20 fn nonsense() -> Int {{\n    1\n  }}\n\
         }}\n"
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::OPERATION_MISMATCH]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("does not declare an operation"));
}

#[test]
fn a_handler_operation_with_the_wrong_arity_is_an_error() {
    // Both parameters used to be accepted and both used to be unknown, so a
    // handler could disagree with its effect about the shape of a call and
    // nobody was told.
    let (sources, checked) = check_source(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to, extra) -> () {{\n    count = 1\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count\n  }}\n\
         }}\n"
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::OPERATION_MISMATCH]
    );
    assert!(
        rendered(&sources, &checked.diagnostics).contains("takes 1 argument, and this takes 2")
    );
}

#[test]
fn a_handler_for_an_imported_effect_gets_its_types_too() {
    check_ok_in(
        "module a\n\n\
         use other.{Ledger}\n\n\
         handler InMemory implements Ledger {\n\
         \x20 state total: Int\n\n\
         \x20 fn post(amount) -> () {\n    total = total + amount\n  }\n\
         }\n",
        &universe_of(&["module other\n\neffect Ledger {\n    fn post(amount: Int) -> ()\n}\n"]),
    );
}

#[test]
fn an_imported_handler_operation_is_checked_against_those_types() {
    let (_, checked) = check_source_in(
        "module a\n\n\
         use other.{Ledger}\n\n\
         handler InMemory implements Ledger {\n\
         \x20 state total: String\n\n\
         \x20 fn post(amount) -> () {\n    total = amount\n  }\n\
         }\n",
        &universe_of(&["module other\n\neffect Ledger {\n    fn post(amount: Int) -> ()\n}\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
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
fn an_imported_type_cannot_take_type_arguments_either() {
    // This used to be accepted, and only because an imported name had no type,
    // so there was nothing for the arguments to be applied to. Vow has no
    // generic declarations, and which module a type was declared in does not
    // change that.
    let (sources, checked) = check_source_in(
        "module a\n\nuse other.{Thing}\n\nfn f(x: Thing<Int>) -> Int { 0 }\n",
        &universe_of(&["module other\n\nrecord Thing { n: Int }\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_GENERIC]);
    assert!(rendered(&sources, &checked.diagnostics).contains("does not take type arguments"));
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
    let resolved = resolve(file, &parsed.module, &Universe::new());
    let checked = check(file, &parsed.module, &resolved.resolutions, &World::new());

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
