//! The Proven tier.
//!
//! Half of these are about what it refuses to prove. A tier that claims more
//! than it settled is worse than one that claims nothing, because the whole
//! point of recording a tier is that a contract never quietly degrades into a
//! runtime check without saying so.

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::{Checked, check_text};
use vow_typeck::Tier;

const POSITIVE: &str = "module a\n\ntype Positive = Int where value > 0\n\n";

fn check(body: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.vow", format!("{POSITIVE}{body}"));
    (sources, checked)
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Asserts the one refinement obligation in `body` landed in `tier`.
fn expect(tier: Tier, body: &str) {
    let (sources, checked) = check(body);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );

    let refinements: Vec<Tier> = checked
        .obligations
        .iter()
        .filter(|obligation| obligation.subject == "Positive" || obligation.subject == "Percent")
        .map(|obligation| obligation.tier)
        .collect();

    assert_eq!(
        refinements,
        vec![tier],
        "obligations: {:?}\n{}",
        checked
            .obligations
            .iter()
            .map(|o| (&o.subject, o.tier))
            .collect::<Vec<_>>(),
        rendered(&sources, &checked.diagnostics)
    );
}

// -- what it proves --------------------------------------------------------

#[test]
fn a_literal_is_still_proven() {
    expect(Tier::Proven, "fn f() -> Positive { 1 }\n");
}

#[test]
fn a_precondition_proves_what_it_implies() {
    expect(
        Tier::Proven,
        "fn f(n: Int) -> Positive\n  where\n    n > 0,\n{\n    n\n}\n",
    );
}

#[test]
fn a_precondition_written_the_other_way_round_works_too() {
    expect(
        Tier::Proven,
        "fn f(n: Int) -> Positive\n  where\n    0 < n,\n{\n    n\n}\n",
    );
}

#[test]
fn a_guard_that_leaves_proves_the_rest_of_the_body() {
    expect(
        Tier::Proven,
        "fn f(n: Int) -> Result<Positive, Int> {\n    if n <= 0 {\n        return err(0)\n    }\n    ok(n)\n}\n",
    );
}

#[test]
fn a_then_branch_knows_its_condition_held() {
    expect(
        Tier::Proven,
        "fn f(n: Int) -> Result<Positive, Int> {\n    if n > 0 {\n        ok(n)\n    } else {\n        err(0)\n    }\n}\n",
    );
}

#[test]
fn an_else_branch_knows_its_condition_failed() {
    expect(
        Tier::Proven,
        "fn f(n: Int) -> Result<Positive, Int> {\n    if n <= 0 {\n        err(0)\n    } else {\n        ok(n)\n    }\n}\n",
    );
}

#[test]
fn a_refined_parameter_is_a_fact_without_being_restated() {
    // Nobody should have to write `where n > 0` next to a `Positive`. Passing
    // one straight through is just type equality and raises no obligation at
    // all, so this goes through arithmetic to make one.
    expect(Tier::Proven, "fn f(n: Positive) -> Positive { n + 0 }\n");
}

#[test]
fn passing_a_refined_value_where_the_same_type_is_wanted_proves_nothing() {
    // Because there is nothing to prove. Worth pinning down: an obligation
    // that does not exist is better than one that is trivially discharged,
    // since the count in `--obligations` is supposed to mean something.
    let (sources, checked) = check("fn f(n: Positive) -> Positive { n }\n");
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert!(checked.obligations.is_empty());
}

#[test]
fn one_refinement_can_reach_another_over_the_same_base() {
    // A `Positive` is not a `NonNegative`, but it widens to `Int` and narrows
    // back, and the predicate that arrives settles the predicate that is
    // wanted. Before this, going sideways was a type error, which meant the
    // only way to have two refinements over `Int` in one program was to never
    // let a value move between them.
    let mut sources = SourceMap::new();
    let checked = check_text(
        &mut sources,
        "test.vow",
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         type NonNegative = Int where value >= 0\n\n\
         fn f(n: Positive) -> NonNegative {\n    n\n}\n"
            .to_string(),
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(Tier::Proven), 1);
}

#[test]
fn going_sideways_the_hard_way_is_still_guarded() {
    // The other direction does not follow, and the tier says so rather than
    // the type checker refusing the program outright.
    let mut sources = SourceMap::new();
    let checked = check_text(
        &mut sources,
        "test.vow",
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         type NonNegative = Int where value >= 0\n\n\
         fn f(n: NonNegative) -> Positive {\n    n\n}\n"
            .to_string(),
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(Tier::Guarded), 1);
}

#[test]
fn two_bounds_prove_a_two_sided_refinement() {
    let (sources, checked) = check(
        "type Percent = Int where value >= 0 && value <= 100\n\nfn f(n: Int) -> Percent\n  where\n    n >= 0,\n    n <= 50,\n{\n    n + 50\n}\n",
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(Tier::Proven), 1);
}

#[test]
fn a_conjunction_in_the_condition_narrows_both_ways() {
    expect(
        Tier::Proven,
        "fn f(n: Int) -> Result<Positive, Int> {\n    if n > 0 && n < 10 {\n        ok(n)\n    } else {\n        err(0)\n    }\n}\n",
    );
}

#[test]
fn a_negated_condition_narrows_too() {
    expect(
        Tier::Proven,
        "fn f(n: Int) -> Result<Positive, Int> {\n    if !(n <= 0) {\n        ok(n)\n    } else {\n        err(0)\n    }\n}\n",
    );
}

// -- what it refuses to prove ----------------------------------------------

#[test]
fn nothing_known_stays_guarded() {
    expect(Tier::Guarded, "fn f(n: Int) -> Positive { n }\n");
}

#[test]
fn a_precondition_that_does_not_imply_it_stays_guarded() {
    expect(
        Tier::Guarded,
        "fn f(n: Int) -> Positive\n  where\n    n > -5,\n{\n    n\n}\n",
    );
}

#[test]
fn arithmetic_that_could_overflow_stays_guarded() {
    // `n` is positive and `n + 1` still is, unless `n` was the largest integer
    // there is. Refusing to prove this is the reasoning working, not a gap.
    expect(Tier::Guarded, "fn f(n: Positive) -> Positive { n + 1 }\n");
}

#[test]
fn a_relationship_between_two_names_is_not_a_fact_an_interval_can_hold() {
    // The largest limitation, and the first one anyone will hit.
    expect(
        Tier::Guarded,
        "fn f(low: Int, high: Int) -> Positive\n  where\n    low < high,\n{\n    high - low\n}\n",
    );
}

#[test]
fn an_ensures_clause_is_a_fact_at_the_call_site() {
    expect(
        Tier::Proven,
        "fn one() -> Int\n  ensures\n    ok  => result == 1,\n{\n    1\n}\n\nfn f() -> Positive { one() }\n",
    );
}

#[test]
fn a_return_type_is_a_fact_at_the_call_site_too() {
    // Nothing is restated in an `ensures`. The type already said it.
    expect(
        Tier::Proven,
        "fn one() -> Positive { 1 }\n\nfn f() -> Positive { one() }\n",
    );
}

#[test]
fn an_ensures_clause_that_relates_the_result_to_an_argument_says_nothing() {
    // True, useful, and not something an interval can hold. The tier says so
    // instead of pretending.
    expect(
        Tier::Guarded,
        "fn same(n: Int) -> Int\n  ensures\n    ok  => result == n,\n{\n    n\n}\n\nfn f(n: Positive) -> Positive { same(n) }\n",
    );
}

#[test]
fn a_call_that_can_fail_promises_nothing_at_the_call_site() {
    // The call site holds a `Result`, not the payload, so the range of the
    // success case is not the range of the expression.
    let (sources, checked) = check(
        "fn one() -> Result<Int, String>\n  ensures\n    ok  => result == 1,\n{\n    ok(1)\n}\n\nfn f() -> Result<Positive, String> { one() }\n",
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(Tier::Proven), 0);
}

#[test]
fn naming_a_value_does_not_lose_what_was_known_about_it() {
    expect(
        Tier::Proven,
        "fn one() -> Positive { 1 }\n\nfn f() -> Positive {\n    let n = one()\n    n\n}\n",
    );
}

#[test]
fn a_fact_does_not_survive_past_the_branch_that_established_it() {
    expect(
        Tier::Guarded,
        "fn f(n: Int) -> Positive {\n    if n > 0 {\n        0\n    } else {\n        0\n    }\n    n\n}\n",
    );
}

#[test]
fn a_fact_does_not_leak_from_one_function_into_another() {
    // Each body is checked against its own contract and nothing else, which is
    // P1. A caller knowing something is not the callee knowing it.
    let (sources, checked) = check(
        "fn caller() -> Int\n  where\n    true,\n{\n    callee(1)\n}\n\nfn callee(n: Int) -> Int { keep(n) }\n\nfn keep(p: Positive) -> Int { p }\n",
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(Tier::Guarded), 1);
    assert_eq!(checked.obligations_at(Tier::Proven), 0);
}

#[test]
fn an_or_says_nothing_about_either_side() {
    expect(
        Tier::Guarded,
        "fn f(n: Int) -> Result<Positive, Int> {\n    if n > 0 || n < -10 {\n        ok(n)\n    } else {\n        err(0)\n    }\n}\n",
    );
}

// -- across a module boundary ----------------------------------------------

/// Checks both files together and returns the result for the second.
fn check_pair(first: &str, second: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let ids = vec![
        sources.add("first.vow", first),
        sources.add("second.vow", second),
    ];
    let mut checks = vow_driver::check_all(&sources, &ids);
    (sources, checks.remove(1))
}

#[test]
fn a_promise_crosses_a_module_boundary() {
    // A refinement is opaque from outside, on purpose, so what travels is the
    // range and not the predicate. The caller gets what it needs to reason
    // without learning how the callee decided anything.
    let (sources, checked) = check_pair(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn one() -> Positive { 1 }\n",
        "module b\n\n\
         use a.{one}\n\n\
         type AlsoPositive = Int where value > 0\n\n\
         fn f() -> AlsoPositive { one() }\n",
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(Tier::Proven), 1);
}

#[test]
fn an_ensures_clause_crosses_a_module_boundary_too() {
    let (sources, checked) = check_pair(
        "module a\n\n\
         fn one() -> Int\n  ensures\n    ok  => result == 1,\n{\n    1\n}\n",
        "module b\n\n\
         use a.{one}\n\n\
         type Positive = Int where value > 0\n\n\
         fn f() -> Positive { one() }\n",
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(Tier::Proven), 1);
}

#[test]
fn a_function_that_promises_nothing_still_promises_nothing_across_a_boundary() {
    let (sources, checked) = check_pair(
        "module a\n\nfn anything() -> Int { 1 }\n",
        "module b\n\n\
         use a.{anything}\n\n\
         type Positive = Int where value > 0\n\n\
         fn f() -> Positive { anything() }\n",
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(Tier::Guarded), 1);
}

// -- what it rejects outright ----------------------------------------------

#[test]
fn a_value_the_facts_rule_out_is_an_error_not_a_guard() {
    let (sources, checked) = check("fn f(n: Int) -> Positive\n  where\n    n < 0,\n{\n    n\n}\n");
    assert!(
        checked
            .diagnostics
            .iter()
            .any(|d| d.code == vow_typeck::codes::VIOLATED_REFINEMENT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_literal_that_violates_it_is_still_an_error() {
    let (sources, checked) = check("fn f() -> Positive { 0 }\n");
    assert!(
        checked
            .diagnostics
            .iter()
            .any(|d| d.code == vow_typeck::codes::VIOLATED_REFINEMENT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- the example -----------------------------------------------------------

#[test]
fn the_proven_example_says_what_it_claims() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/proven.vow");
    let source = std::fs::read_to_string(path).expect("examples/proven.vow should exist");

    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "examples/proven.vow", source);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );

    // Nine proven and three guarded, and the file explains each of the three.
    // If either number moves, the comments in the example are wrong.
    assert_eq!(checked.obligations_at(Tier::Proven), 9);
    assert_eq!(checked.obligations_at(Tier::Guarded), 3);
}
