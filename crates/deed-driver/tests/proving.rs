//! The Proven tier.
//!
//! Half of these are about what it refuses to prove. A tier that claims more
//! than it settled is worse than one that claims nothing, because the whole
//! point of recording a tier is that a contract never quietly degrades into a
//! runtime check without saying so.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::{Checked, check_text};
use deed_typeck::Tier;

const POSITIVE: &str = "module a\n\ntype Positive = Int where value > 0\n\ntype NonEmpty = String where length(value) > 0\n\ntype NonEmptyList = List<Int> where length(value) > 0\n\n";

fn check(body: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.deed", format!("{POSITIVE}{body}"));
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
        .filter(|obligation| {
            matches!(
                obligation.subject.as_str(),
                "Positive" | "Percent" | "Negative" | "NonNegative" | "NonEmpty" | "NonEmptyList"
            )
        })
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

/// Asserts the obligations `body` raised are exactly `wanted`, in source order.
///
/// The general form of `expect`, for a body that raises more than one or
/// raises something other than a refinement. It exists so that such a test has
/// somewhere to go other than a hand-written loop: the one below this used to
/// say `all(|tier| *tier == Proven)` over a list nothing had said was not
/// empty, and an empty list satisfies that for free.
fn expect_each(wanted: &[(&str, Tier)], body: &str) {
    let (sources, checked) = check(body);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );

    let raised: Vec<(&str, Tier)> = checked
        .obligations
        .iter()
        .map(|obligation| (obligation.subject.as_str(), obligation.tier))
        .collect();

    assert_eq!(
        raised,
        wanted,
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- how long the thing is -------------------------------------------------
//
// A predicate about a length used to be unsettleable. `value` had no
// definition, so it was carried as a range, and a range answers `value > 0`
// and cannot answer `length(value) > 0`, which is a question about a term. So
// a `where` clause and a refinement saying the same thing gave two different
// answers about the same guard.

/// Takes a string that has to be non-empty, and a `where` clause saying it a
/// second way, so the two can be compared on the same guard.
const TAKES: &str = "fn take(s: NonEmpty) -> Int {\n    length(s)\n}\n\n\
     fn required(s: String) -> Int\n\
     \x20 where\n\
     \x20   length(s) > 0,\n\
     {\n\
     \x20   length(s)\n\
     }\n\n";

#[test]
fn a_guard_on_a_length_proves_a_refinement_about_it() {
    expect(
        Tier::Proven,
        &format!(
            "{TAKES}fn f(s: String) -> Int {{\n\
             \x20   if length(s) > 0 {{\n\
             \x20       take(s)\n\
             \x20   }} else {{\n\
             \x20       0\n\
             \x20   }}\n\
             }}\n"
        ),
    );
}

#[test]
fn a_length_nothing_settled_is_still_guarded() {
    expect(
        Tier::Guarded,
        &format!("{TAKES}fn f(s: String) -> Int {{\n    take(s)\n}}\n"),
    );
}

/// The whole point. A `where` clause and a refinement say the same thing about
/// the same guard, and they used to disagree: the call site read the caller's
/// facts for one and a bare range for the other.
///
/// Both are named here rather than asserted over. The version that collected
/// the tiers and asked whether they were all `Proven` was satisfied by no
/// obligations at all, so the one test in this file that is about two of them
/// agreeing was the one that could not tell whether either had happened.
#[test]
fn a_where_clause_and_a_refinement_agree_about_a_length() {
    expect_each(
        &[
            ("required requires", Tier::Proven),
            ("NonEmpty", Tier::Proven),
        ],
        &format!(
            "{TAKES}fn f(s: String) -> Int {{\n\
             \x20   if length(s) > 0 {{\n\
             \x20       required(s) + take(s)\n\
             \x20   }} else {{\n\
             \x20       0\n\
             \x20   }}\n\
             }}\n"
        ),
    );
}

/// A string written on the spot has a length nobody has to work out, and
/// refusing to count it would be refusing a fact for want of somewhere to put
/// it. That was the other half of the same gap: a literal has no name either.
#[test]
fn a_string_written_on_the_spot_says_its_own_length() {
    expect(
        Tier::Proven,
        &format!("{TAKES}fn f() -> Int {{\n    take(\"hi\")\n}}\n"),
    );
}

#[test]
fn a_string_written_on_the_spot_that_cannot_satisfy_it_is_refused() {
    // Not guarded. The checker can see this one fail, and a runtime check for
    // something already known is a check nobody should have had to run.
    let (sources, checked) = check(&format!("{TAKES}fn f() -> Int {{\n    take(\"\")\n}}\n"));
    assert!(checked.has_errors());
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("does not satisfy `NonEmpty`"), "{text}");
}

/// Only a bare name. `f(x)` produces a value nothing names, and two calls that
/// looked like one term would be worse than no term.
#[test]
fn a_length_of_something_a_call_returned_is_not_a_term() {
    expect(
        Tier::Guarded,
        &format!(
            "{TAKES}fn twice(s: String) -> String {{\n    s + s\n}}\n\n\
             fn f(s: String) -> Int {{\n\
             \x20   if length(twice(s)) > 0 {{\n\
             \x20       take(twice(s))\n\
             \x20   }} else {{\n\
             \x20       0\n\
             \x20   }}\n\
             }}\n"
        ),
    );
}

// -- a refinement over a list ----------------------------------------------

const TAKES_LIST: &str = "fn take_list(items: NonEmptyList) -> Int {\n    length(items)\n}\n\n";

#[test]
fn a_list_written_on_the_spot_says_its_own_length() {
    expect(
        Tier::Proven,
        &format!("{TAKES_LIST}fn f() -> Int {{\n    take_list([1, 2, 3])\n}}\n"),
    );
}

#[test]
fn a_guard_on_a_list_length_proves_a_refinement_about_it() {
    expect(
        Tier::Proven,
        &format!(
            "{TAKES_LIST}fn f(items: List<Int>) -> Int {{\n\
             \x20   if length(items) > 0 {{\n\
             \x20       take_list(items)\n\
             \x20   }} else {{\n\
             \x20       0\n\
             \x20   }}\n\
             }}\n"
        ),
    );
}

/// The empty list is a `List<unknown>` and fits a `List<Int>`, so it fits the
/// base of a refinement over one. What is wrong with it is the predicate. The
/// refinement branch used to ask for the base type exactly, so this came back
/// as "expected `NonEmptyList`, found `List<_>`", which is a true sentence
/// about the wrong problem.
#[test]
fn an_empty_list_is_refused_by_the_predicate_rather_than_by_the_type() {
    let (sources, checked) = check(&format!(
        "{TAKES_LIST}fn f() -> Int {{\n    take_list([])\n}}\n"
    ));
    assert!(checked.has_errors());
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("does not satisfy `NonEmptyList`"), "{text}");
    assert!(!text.contains("found `List<_>`"), "{text}");
}

/// A parameter already of a refined type is a fact without a `where` clause
/// repeating the type in prose. That held for what the value is worth and not
/// for how long it is, so a `NonEmptyList` knew nothing about its own length
/// inside the body that declared it.
#[test]
fn a_parameter_refined_over_its_length_knows_it() {
    expect(
        Tier::Proven,
        "fn f(items: NonEmptyList) -> Positive {\n    length(items)\n}\n",
    );
}

#[test]
fn the_same_holds_for_a_string() {
    expect(
        Tier::Proven,
        "fn f(s: NonEmpty) -> Positive {\n    length(s)\n}\n",
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
        "test.deed",
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
        "test.deed",
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

#[test]
fn a_relationship_between_two_names_is_a_fact() {
    // What `low < high` says is nothing about either name and everything about
    // the difference, so there is a range for the difference next to the range
    // for each name. The bounds are there because the subtraction has to have
    // an answer, not because the relationship needs them.
    expect(
        Tier::Proven,
        "fn f(low: Int, high: Int) -> Positive\n  where\n    low >= 0,\n    high <= 100,\n    low < high,\n{\n    high - low\n}\n",
    );
}

#[test]
fn the_relationship_reads_the_same_written_backwards() {
    expect(
        Tier::Proven,
        "fn f(low: Int, high: Int) -> Positive\n  where\n    low >= 0,\n    high <= 100,\n    high > low,\n{\n    high - low\n}\n",
    );
}

#[test]
fn a_relationship_with_an_offset_counts_too() {
    // `low + 1 < high` is `low - high < -1`, so the difference is at least two.
    expect(
        Tier::Proven,
        "type Percent = Int where value >= 0 && value <= 100\n\nfn f(low: Int, high: Int) -> Percent\n  where\n    low >= 0,\n    high <= 100,\n    low + 1 < high,\n{\n    high - low - 2\n}\n",
    );
}

#[test]
fn two_relationships_that_share_a_name_make_a_third() {
    expect(
        Tier::Proven,
        "fn f(a: Int, b: Int, c: Int) -> Positive\n  where\n    a >= 0,\n    c <= 100,\n    a < b,\n    b < c,\n{\n    c - a\n}\n",
    );
}

#[test]
fn a_relationship_tightens_what_is_known_about_a_name() {
    // The bound on `limit` arrives after the comparison that needs it, so
    // narrowing `n` against `limit` where it stands cannot see it. The
    // difference can: once `limit` is settled, `n` follows from `n - limit`.
    expect(
        Tier::Proven,
        "type Percent = Int where value >= 0 && value <= 100\n\nfn f(n: Int, limit: Int) -> Percent\n  where\n    n < limit,\n    limit <= 101,\n    n >= 0,\n{\n    n\n}\n",
    );
}

#[test]
fn an_equality_carries_a_bound_from_one_name_to_the_other() {
    // The clause that bounds `a` comes after the one that ties it to `b`, so
    // nothing is in scope to narrow `b` with at the point it is written. The
    // difference is what carries it, once there is something to carry.
    expect(
        Tier::Proven,
        "fn f(a: Int, b: Int) -> Positive\n  where\n    a == b,\n    a > 0,\n{\n    b\n}\n",
    );
}

#[test]
fn an_ensures_clause_that_ties_the_result_to_an_argument_travels() {
    // The ordinary shape of a promise, and it says nothing as a pair of bounds.
    // What crosses the call is `result - n`, which is zero here, so the
    // caller's `n` being positive makes the result positive.
    expect(
        Tier::Proven,
        "fn same(n: Int) -> Int\n  ensures\n    ok  => result == n,\n{\n    n\n}\n\nfn f(n: Positive) -> Positive { same(n) }\n",
    );
}

#[test]
fn a_promise_can_be_an_inequality() {
    expect(
        Tier::Proven,
        "fn at_least(n: Int) -> Int\n  ensures\n    ok  => result >= n,\n{\n    n\n}\n\nfn f(n: Positive) -> Positive { at_least(n) }\n",
    );
}

#[test]
fn a_promise_can_move_the_result_away_from_the_argument() {
    expect(
        Tier::Proven,
        "fn next(n: Int) -> Int\n  ensures\n    ok  => result == n + 1,\n{\n    n + 1\n}\n\nfn f(n: Int) -> Positive\n  where\n    n >= 0,\n{\n    next(n)\n}\n",
    );
}

#[test]
fn a_promise_is_about_the_argument_in_that_position() {
    // Two parameters, and the promise names the second. Getting the position
    // wrong would prove this from whatever happened to be first.
    expect(
        Tier::Proven,
        "fn second(a: Int, b: Int) -> Int\n  ensures\n    ok  => result == b,\n{\n    b\n}\n\nfn f(a: Int, b: Positive) -> Positive { second(a, b) }\n",
    );
}

#[test]
fn two_clauses_bound_the_result_from_both_sides() {
    expect(
        Tier::Proven,
        "type Percent = Int where value >= 0 && value <= 100\n\nfn near(n: Int) -> Int\n  ensures\n    ok  => result >= n,\n    ok  => result <= n + 50,\n{\n    n\n}\n\nfn f(n: Int) -> Percent\n  where\n    n >= 0,\n    n <= 50,\n{\n    near(n)\n}\n",
    );
}

#[test]
fn a_name_counted_twice_is_still_a_name() {
    // `result == n + n` is two of one name, which is not a difference and is
    // not a product either. Refusing it would be refusing the clause over how
    // it happened to be written.
    expect(
        Tier::Proven,
        "fn doubled(n: Int) -> Int\n  ensures\n    ok  => result == n + n,\n{\n    n + n\n}\n\nfn f(n: Positive) -> Positive { doubled(n) }\n",
    );
}

#[test]
fn a_name_multiplied_by_a_number_is_still_a_name() {
    expect(
        Tier::Proven,
        "fn doubled(n: Int) -> Int\n  ensures\n    ok  => result == n * 2,\n{\n    n * 2\n}\n\nfn f(n: Positive) -> Positive { doubled(n) }\n",
    );
}

#[test]
fn a_clause_can_count_a_name_on_either_side() {
    expect(
        Tier::Proven,
        "fn doubled(n: Int) -> Int\n  ensures\n    ok  => 2 * n == result,\n{\n    n + n\n}\n\nfn f(n: Positive) -> Positive { doubled(n) }\n",
    );
}

#[test]
fn a_bound_on_a_multiple_is_a_bound_on_the_name() {
    // `n * 3 > 0` and `n * 3 < 100` put `n` between one and thirty three, and
    // neither has a bare name on one side for the interval to narrow.
    expect(
        Tier::Proven,
        "fn f(n: Int) -> Positive\n  where\n    n * 3 > 0,\n    n * 3 < 100,\n{\n    n\n}\n",
    );
}

#[test]
fn dividing_a_bound_rounds_towards_the_smaller_range() {
    // `3 * n <= -2` admits `n <= -1`, not `n <= 0`: rounding towards zero here
    // would admit a value that does not satisfy the constraint at all.
    expect(
        Tier::Proven,
        "type Negative = Int where value < 0\n\nfn f(n: Int) -> Negative\n  where\n    n * 3 <= -2,\n{\n    n\n}\n",
    );
}

#[test]
fn a_bound_on_a_multiple_that_does_not_divide_evenly_is_still_exact() {
    // `2 * n > 0` means `2 * n >= 1`, and the smallest integer `n` with
    // `2 * n >= 1` is one, not zero.
    expect(
        Tier::Proven,
        "fn f(n: Int) -> Positive\n  where\n    n * 2 > 0,\n{\n    n\n}\n",
    );
}

// -- how long something is -------------------------------------------------
//
// `length(items)` used to come back as a range and nothing more, so it could
// not be one side of a difference and `index < length(items)` was a shape the
// relation machinery could not see. It is a term now, keyed on the thing being
// measured, and everything below falls out of the two rules that were already
// there rather than out of anything written for lengths.

#[test]
fn a_length_is_not_negative_without_a_where_clause_saying_so() {
    expect(
        Tier::Proven,
        "type NonNegative = Int where value >= 0\n\n\
         fn f(items: List<Int>) -> NonNegative { length(items) }\n",
    );
}

#[test]
fn a_checked_length_proves_the_index_below_it() {
    // The one this was for. Past the guard the list holds something, so one
    // less than its length is not negative.
    expect(
        Tier::Proven,
        "type NonNegative = Int where value >= 0\n\n\
         fn f(items: List<Int>) -> Result<NonNegative, Int> {\n\
         \x20 if length(items) <= 0 {\n\
         \x20   return err(0)\n\
         \x20 }\n\
         \x20 ok(length(items) - 1)\n\
         }\n",
    );
}

#[test]
fn an_unchecked_length_proves_nothing_about_the_index_below_it() {
    // The other half. An empty list has a length of zero and one less than
    // that is negative, so this is a runtime check and says so.
    expect(
        Tier::Guarded,
        "type NonNegative = Int where value >= 0\n\n\
         fn f(items: List<Int>) -> NonNegative { length(items) - 1 }\n",
    );
}

#[test]
fn a_precondition_about_a_length_is_a_fact_about_the_body() {
    // A `where` clause narrows the same term an `if` does, so where the bound
    // came from does not change what follows from it.
    expect(
        Tier::Proven,
        "type NonNegative = Int where value >= 0\n\n\
         fn f(items: List<Int>) -> NonNegative\n\
         \x20 where\n\
         \x20   length(items) >= 1,\n\
         {\n\
         \x20 length(items) - 1\n\
         }\n",
    );
}

#[test]
fn an_index_below_a_length_is_a_difference_like_any_other() {
    // The shape this exists for, and the reason it needed a term rather than a
    // range: `low < high` and `index < length(items)` are one rule.
    expect(
        Tier::Proven,
        "type NonNegative = Int where value >= 0\n\n\
         fn f(items: List<Int>, index: Int) -> NonNegative\n\
         \x20 where\n\
         \x20   index >= 0,\n\
         \x20   index < length(items),\n\
         {\n\
         \x20 length(items) - index\n\
         }\n",
    );
}

#[test]
fn a_string_has_a_length_the_same_way_a_list_does() {
    // `length` was about strings before there were lists and it stayed one
    // name, so the term is one thing too.
    expect(
        Tier::Proven,
        "type NonNegative = Int where value >= 0\n\n\
         fn f(text: String) -> Result<NonNegative, Int> {\n\
         \x20 if length(text) <= 0 {\n\
         \x20   return err(0)\n\
         \x20 }\n\
         \x20 ok(length(text) - 1)\n\
         }\n",
    );
}

#[test]
fn two_lists_have_two_lengths() {
    // Keyed on the thing being measured, so a bound on one says nothing about
    // the other. Getting this wrong is the way a term like this goes bad.
    expect(
        Tier::Guarded,
        "type NonNegative = Int where value >= 0\n\n\
         fn f(items: List<Int>, others: List<Int>) -> Result<NonNegative, Int> {\n\
         \x20 if length(items) <= 0 {\n\
         \x20   return err(0)\n\
         \x20 }\n\
         \x20 ok(length(others) - 1)\n\
         }\n",
    );
}

#[test]
fn a_length_of_something_that_is_not_a_name_is_not_a_term() {
    // `length(g(items))` names nothing that stays put: two calls could hand
    // back two different lists, so a fact about one of them is not a fact
    // about the other.
    expect(
        Tier::Guarded,
        "type NonNegative = Int where value >= 0\n\n\
         fn g(items: List<Int>) -> List<Int> { items }\n\n\
         fn f(items: List<Int>) -> Result<NonNegative, Int> {\n\
         \x20 if length(g(items)) <= 0 {\n\
         \x20   return err(0)\n\
         \x20 }\n\
         \x20 ok(length(g(items)) - 1)\n\
         }\n",
    );
}

#[test]
fn a_length_recorded_against_reassigned_handler_state_does_not_outlive_it() {
    // Handler state is the only thing that can be assigned twice, and it is
    // where a fact about a list's length could outlive the list. The same
    // mistake was made once already with an integer.
    expect(
        Tier::Guarded,
        "type NonNegative = Int where value >= 0\n\n\
         effect Store {\n\
         \x20 fn keep(item: Int) -> Int\n\
         }\n\n\
         handler Kept implements Store {\n\
         \x20 state items: List<Int>\n\n\
         \x20 fn keep(item: Int) -> Int {\n\
         \x20   let sized: NonNegative = if length(items) > 0 {\n\
         \x20     items = []\n\
         \x20     length(items) - 1\n\
         \x20   } else {\n\
         \x20     0\n\
         \x20   }\n\
         \x20   sized\n\
         \x20 }\n\
         }\n",
    );
}

// -- what it refuses to prove ----------------------------------------------

#[test]
fn a_difference_that_could_leave_the_integers_is_still_proven() {
    // `low < high` is held, so the difference is at least one, and it does not
    // matter that `high - low` can be larger than an integer: overflow is an
    // error rather than a wrap, so a difference that exists is inside `i64`
    // and the range saturates at the edge rather than collapsing.
    expect(
        Tier::Proven,
        "fn f(low: Int, high: Int) -> Positive\n  where\n    low < high,\n{\n    high - low\n}\n",
    );
}

#[test]
fn a_relationship_that_does_not_imply_it_stays_guarded() {
    // `<=` leaves the two names free to be equal, so the difference reaches
    // zero and `Positive` does not follow.
    expect(
        Tier::Guarded,
        "fn f(low: Int, high: Int) -> Positive\n  where\n    low >= 0,\n    high <= 100,\n    low <= high,\n{\n    high - low\n}\n",
    );
}

#[test]
fn a_relationship_does_not_survive_the_branch_that_established_it() {
    expect(
        Tier::Guarded,
        "fn f(low: Int, high: Int) -> Positive\n  where\n    low >= 0,\n    high <= 100,\n{\n    if low < high {\n        0\n    } else {\n        0\n    }\n    high - low\n}\n",
    );
}

#[test]
fn a_relationship_through_a_product_is_not_a_difference() {
    // Two names related by anything other than adding and subtracting is a
    // solver's job, and P9 has a budget against having one at check time.
    expect(
        Tier::Guarded,
        "fn f(a: Int, b: Int) -> Positive\n  where\n    a >= 0,\n    a <= 100,\n    b >= 0,\n    b <= 100,\n    a < b * b,\n{\n    b * b - a\n}\n",
    );
}

#[test]
fn an_or_says_nothing_about_a_relationship_either() {
    expect(
        Tier::Guarded,
        "fn f(low: Int, high: Int) -> Positive\n  where\n    low >= 0,\n    high <= 100,\n    low < high || low > high,\n{\n    high - low\n}\n",
    );
}

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
fn arithmetic_that_could_overflow_is_still_proven() {
    // `n` is positive, so `n + 1` is too. It does not matter that the sum has
    // no answer when `n` is the largest integer there is: overflow stops the
    // program rather than wrapping it, so a value that exists is inside `i64`
    // and every value this can produce is greater than one.
    //
    // This used to be Guarded, and the example that explained why was arguing
    // for a runtime check that could never have fired.
    expect(Tier::Proven, "fn f(n: Positive) -> Positive { n + 1 }\n");
}

#[test]
fn a_bound_that_leaves_the_integers_clamps_rather_than_collapsing() {
    // The same rule read from the other end. `n + 1` for an unbounded `n` can
    // produce anything from `MIN + 1` up, and `MIN + 1` is not positive, so
    // this is Guarded for a real reason rather than because the arithmetic was
    // given up on.
    expect(Tier::Guarded, "fn f(n: Int) -> Positive { n + 1 }\n");
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
fn a_promise_through_a_product_is_not_a_difference() {
    // `result == n * n` is true and useful and there is nowhere to put it,
    // which is the same limit as `a < b * b` inside a body.
    expect(
        Tier::Guarded,
        "fn square(n: Int) -> Int\n  ensures\n    ok  => result == n * n,\n{\n    n * n\n}\n\nfn f(n: Positive) -> Positive { square(n) }\n",
    );
}

#[test]
fn a_promise_about_an_argument_nobody_bounded_says_nothing() {
    // The difference travels, and a difference from an unknown number is an
    // unknown number.
    expect(
        Tier::Guarded,
        "fn same(n: Int) -> Int\n  ensures\n    ok  => result == n,\n{\n    n\n}\n\nfn f(n: Int) -> Positive { same(n) }\n",
    );
}

#[test]
fn a_promise_about_the_failure_case_says_nothing_about_the_value() {
    expect(
        Tier::Guarded,
        "fn same(n: Int) -> Int\n  ensures\n    err => result == n,\n{\n    n\n}\n\nfn f(n: Positive) -> Positive { same(n) }\n",
    );
}

#[test]
fn a_promise_on_a_call_that_can_fail_reaches_the_payload() {
    // The call site holds a `Result` and the promise was about the number
    // inside it, so the two only meet where the payload comes out. Passing a
    // whole `Result` into one with a refined success type is one such place.
    let (sources, checked) = check(
        "fn one() -> Result<Int, String>\n  ensures\n    ok  => result == 1,\n{\n    ok(1)\n}\n\nfn f() -> Result<Positive, String> { one() }\n",
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(Tier::Proven), 1);
}

#[test]
fn a_promise_reaches_through_a_question_mark() {
    // The ordinary way of using a function that can fail, and it used to throw
    // the contract away.
    expect(
        Tier::Proven,
        "fn one() -> Result<Int, String>\n  ensures\n    ok  => result == 1,\n{\n    ok(1)\n}\n\nfn f() -> Result<Positive, String> {\n    let n = one()?\n    ok(n)\n}\n",
    );
}

#[test]
fn a_promise_reaches_the_name_an_ok_pattern_binds() {
    expect(
        Tier::Proven,
        "fn one() -> Result<Int, String>\n  ensures\n    ok  => result == 1,\n{\n    ok(1)\n}\n\nfn f() -> Result<Positive, String> {\n    match one() {\n        ok(n) => ok(n),\n        err(e) => err(e),\n    }\n}\n",
    );
}

#[test]
fn a_call_that_can_fail_and_promises_nothing_still_promises_nothing() {
    let (sources, checked) = check(
        "fn one() -> Result<Int, String> { ok(1) }\n\nfn f() -> Result<Positive, String> { one() }\n",
    );
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(Tier::Guarded), 1);
}

#[test]
fn a_promise_about_the_success_case_says_nothing_about_the_failure_one() {
    let (sources, checked) = check(
        "fn one() -> Result<Int, Int>\n  ensures\n    ok  => result == 1,\n{\n    ok(1)\n}\n\nfn f() -> Result<Int, Positive> { one() }\n",
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

// -- what a caller has to answer for ---------------------------------------
//
// `design/02-syntax.md` has always said a precondition is checked at the call
// site when it can be proven there. It was not. A `where` clause was a fact for
// the callee's body and a check inside the callee at runtime, and nothing ever
// looked at it from where the call was written, so a call that plainly broke
// the contract passed the checker in silence.
//
// The runtime check stays whatever happens here, the same way an `ensures`
// clause runs on every call whatever tier it landed in.

/// A function nobody may call with a negative number.
const HALVE: &str = "fn halve(n: Int) -> Int\n\
     \x20 where\n\
     \x20   n >= 0,\n\
     {\n\
     \x20 n\n\
     }\n\n";

/// The tiers of every precondition in `body`, in order.
fn preconditions(body: &str) -> (Vec<Tier>, SourceMap, Checked) {
    let (sources, checked) = check(body);
    let tiers = checked
        .obligations
        .iter()
        .filter(|obligation| obligation.subject.ends_with(" requires"))
        .map(|obligation| obligation.tier)
        .collect();
    (tiers, sources, checked)
}

#[test]
fn a_call_that_plainly_breaks_a_precondition_is_refused() {
    let (_, sources, checked) =
        preconditions(&format!("{HALVE}fn caller() -> Int {{ halve(0 - 5) }}\n"));
    let text = rendered(&sources, &checked.diagnostics);
    assert!(checked.has_errors(), "this should not have been accepted");
    assert!(text.contains("DEED4025"), "{text}");
    assert!(
        text.contains("does not satisfy what `halve` requires"),
        "{text}"
    );
}

#[test]
fn a_caller_that_can_show_it_holds_proves_it() {
    let (tiers, sources, checked) = preconditions(&format!(
        "{HALVE}fn caller(n: Int) -> Int\n\
         \x20 where\n\
         \x20   n > 3,\n\
         {{\n\
         \x20 halve(n)\n\
         }}\n"
    ));
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(tiers, vec![Tier::Proven]);
}

#[test]
fn a_guard_that_leaves_proves_the_call_after_it() {
    let (tiers, _, _) = preconditions(&format!(
        "{HALVE}fn caller(n: Int) -> Int {{\n\
         \x20 if n < 0 {{\n\
         \x20   return 0\n\
         \x20 }}\n\
         \x20 halve(n)\n\
         }}\n"
    ));
    assert_eq!(tiers, vec![Tier::Proven]);
}

#[test]
fn a_caller_that_knows_nothing_leaves_it_to_the_runtime() {
    // Not knowing is the ordinary case and is not a mistake. The check inside
    // the callee is still there, so this is a tier and not a hole.
    let (tiers, sources, checked) =
        preconditions(&format!("{HALVE}fn caller(n: Int) -> Int {{ halve(n) }}\n"));
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(tiers, vec![Tier::Guarded]);
}

#[test]
fn a_precondition_about_two_arguments_reads_the_pair() {
    // The clause is about `low` and `high` together, so what has to cross into
    // it is the difference between the two arguments and not a bound on
    // either.
    let (tiers, _, _) = preconditions(
        "fn between(low: Int, high: Int) -> Int\n\
         \x20 where\n\
         \x20   low < high,\n\
         {\n\
         \x20 high - low\n\
         }\n\n\
         fn caller(a: Int, b: Int) -> Int\n\
         \x20 where\n\
         \x20   a < b,\n\
         {\n\
         \x20 between(a, b)\n\
         }\n",
    );
    assert_eq!(tiers, vec![Tier::Proven]);
}

#[test]
fn a_precondition_about_a_length_is_settled_by_a_caller_that_checked_it() {
    // What #142 was for. The clause names a length, the caller checked one,
    // and the two meet because a length is a term on both sides.
    let (tiers, _, _) = preconditions(
        "fn nth(items: List<Int>, index: Int) -> Result<Int, String>\n\
         \x20 where\n\
         \x20   index >= 0,\n\
         \x20   index < length(items),\n\
         {\n\
         \x20 at(items, index)\n\
         }\n\n\
         fn head(items: List<Int>) -> Result<Int, String> {\n\
         \x20 if length(items) <= 0 {\n\
         \x20   return err(\"empty\")\n\
         \x20 }\n\
         \x20 nth(items, 0)\n\
         }\n",
    );
    assert_eq!(tiers, vec![Tier::Proven, Tier::Proven]);
}

#[test]
fn a_list_written_on_the_spot_knows_how_long_it_is() {
    let (tiers, _, _) = preconditions(
        "fn nth(items: List<Int>, index: Int) -> Result<Int, String>\n\
         \x20 where\n\
         \x20   index >= 0,\n\
         \x20   index < length(items),\n\
         {\n\
         \x20 at(items, index)\n\
         }\n\n\
         fn second() -> Result<Int, String> { nth([1, 2, 3], 1) }\n",
    );
    assert_eq!(tiers, vec![Tier::Proven, Tier::Proven]);
}

#[test]
fn an_index_off_the_end_of_a_list_written_on_the_spot_is_refused() {
    let (_, sources, checked) = preconditions(
        "fn nth(items: List<Int>, index: Int) -> Result<Int, String>\n\
         \x20 where\n\
         \x20   index >= 0,\n\
         \x20   index < length(items),\n\
         {\n\
         \x20 at(items, index)\n\
         }\n\n\
         fn fourth() -> Result<Int, String> { nth([1, 2, 3], 3) }\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(checked.has_errors(), "this should not have been accepted");
    assert!(text.contains("DEED4025"), "{text}");
}

#[test]
fn a_precondition_crosses_a_module_boundary() {
    // It did not, and that was the one place the rule this whole section is
    // about had a hole in it. A clause was a fact for the callee's body and a
    // check inside the callee at runtime, and an imported signature arrived
    // with no clause on it at all, so the same broken call was a refused
    // mistake at home and silence one file away.
    //
    // A precondition is a question the caller has to answer and the caller is
    // the only one who can answer it, so it crosses whole. A refinement
    // predicate still does not, for the reason at the top of `surface.rs`.

    // The same call in one module first, so that what happens below is the
    // boundary rather than the obligation never existing at all. Without it a
    // compiler that stopped reporting preconditions entirely would pass here.
    let (_, together) = check(
        "fn halve(n: Int) -> Int\n\
         \x20 where\n\
         \x20   n >= 0,\n\
         {\n\
         \x20 n\n\
         }\n\n\
         fn caller() -> Int { halve(5) }\n",
    );
    assert_eq!(
        together
            .obligations
            .iter()
            .map(|o| (o.subject.as_str(), o.tier))
            .collect::<Vec<_>>(),
        vec![("halve requires", Tier::Proven)]
    );

    let mut sources = SourceMap::new();
    let ids: Vec<_> = [
        "module app\n\n\
         use lib.{halve}\n\n\
         fn caller() -> Int { halve(0 - 5) }\n",
        "module lib\n\n\
         fn halve(n: Int) -> Int\n\
         \x20 where\n\
         \x20   n >= 0,\n\
         {\n\
         \x20 n\n\
         }\n",
    ]
    .iter()
    .enumerate()
    .map(|(index, text)| sources.add(format!("file{index}.deed"), *text))
    .collect();
    let mut checks = deed_driver::check_all(&sources, &ids);
    let checked = checks.remove(0);

    let text = rendered(&sources, &checked.diagnostics);
    assert!(checked.has_errors(), "this should not have been accepted");
    assert!(text.contains("DEED4025"), "{text}");
    assert!(
        text.contains("does not satisfy what `halve` requires"),
        "{text}"
    );
}

// -- why a proof failed ----------------------------------------------------

#[test]
fn a_sum_is_never_blamed_for_having_no_answer() {
    // It used to be, and that note was explaining a refusal that should not
    // have happened. Overflow is an error rather than a wrap, so a sum that
    // produced a value produced one inside `i64`, and the only arithmetic that
    // can still defeat a proof by having no answer is dividing.
    let (sources, checked) = check("fn f(n: Int) -> Positive { n + 1 }\n");
    let text = rendered(&sources, &checked.diagnostics);
    assert!(!text.contains("this can have no answer"), "{text}");
}

#[test]
fn dividing_by_something_that_could_be_zero_is_named() {
    // The one piece of arithmetic that can still defeat a proof by having no
    // answer, and a reader looking at a refusal here has every right to think
    // the reasoning is weak unless it says which operation.
    let (sources, checked) = check("fn f(n: Positive, d: Int) -> Positive { n / d }\n");
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("this can have no answer"), "{text}");
    assert!(text.contains("bounding what goes into it"), "{text}");
}

#[test]
fn a_proof_nothing_defeated_does_not_blame_the_arithmetic() {
    // A refinement nobody said anything about is a different failure from one
    // the arithmetic ruled out, and printing the note over both would make it
    // worth nothing.
    let (sources, checked) = check("fn f(n: Int) -> Positive { n }\n");
    let text = rendered(&sources, &checked.diagnostics);
    assert!(!text.contains("this can have no answer"), "{text}");
}

#[test]
fn arithmetic_that_cannot_overflow_is_not_blamed() {
    let (sources, checked) = check(
        "type Percent = Int where value >= 0 && value <= 100\n\nfn f(n: Int) -> Percent\n  where\n    n >= 0,\n    n <= 50,\n{\n    n + 60\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(!text.contains("this can have no answer"), "{text}");
}

// -- across a module boundary ----------------------------------------------

/// Checks both files together and returns the result for the second.
fn check_pair(first: &str, second: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let ids = vec![
        sources.add("first.deed", first),
        sources.add("second.deed", second),
    ];
    let mut checks = deed_driver::check_all(&sources, &ids);
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
fn a_promise_about_an_argument_crosses_a_module_boundary() {
    // The predicate stays behind, which is the rule modules already had. What
    // arrives is a range and the difference from each argument, which is a pair
    // of numbers per argument rather than an expression out of someone else's
    // scope.
    let (sources, checked) = check_pair(
        "module a\n\n\
         fn same(n: Int) -> Int\n  ensures\n    ok  => result == n,\n{\n    n\n}\n",
        "module b\n\n\
         use a.{same}\n\n\
         type Positive = Int where value > 0\n\n\
         fn f(n: Positive) -> Positive { same(n) }\n",
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
            .any(|d| d.code == deed_typeck::codes::VIOLATED_REFINEMENT),
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
            .any(|d| d.code == deed_typeck::codes::VIOLATED_REFINEMENT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- the example -----------------------------------------------------------

#[test]
fn the_proven_example_says_what_it_claims() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/proven.deed");
    let source = std::fs::read_to_string(path).expect("examples/proven.deed should exist");

    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "examples/proven.deed", source);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );

    // Forty-eight proven and two guarded, and the file explains both. If
    // either number moves, the comments in the example are wrong.
    assert_eq!(checked.obligations_at(Tier::Proven), 48);
    assert_eq!(checked.obligations_at(Tier::Guarded), 2);
}

#[test]
fn a_guard_weaker_than_the_predicate_says_what_gets_through() {
    // The gap `design/02-syntax.md` names under open questions: a converter
    // guarded by `n >= 0` for a `value > 0` refinement is accepted with a
    // runtime check, which is honest and is not the same as catching it. It
    // stays accepted, because a runtime check is what `Guarded` means, and the
    // diagnostic now names the number the guard lets past.
    let (sources, checked) = check(
        "\
         fn try_positive(n: Int) -> Result<Positive, String> {\n\
         \x20   if n >= 0 {\n        ok(n)\n    } else {\n        err(\"no\")\n    }\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("cannot prove this satisfies `Positive`"),
        "{text}"
    );
    assert!(
        text.contains("when this is 0 it does not satisfy `Positive`"),
        "{text}"
    );
}

#[test]
fn a_guard_that_matches_the_predicate_proves_it_and_says_nothing() {
    // The other side, and the one that says the note above is about the guard
    // rather than about every conversion. One character apart.
    let (sources, checked) = check(
        "\
         fn try_positive(n: Int) -> Result<Positive, String> {\n\
         \x20   if n > 0 {\n        ok(n)\n    } else {\n        err(\"no\")\n    }\n}\n",
    );
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(checked.obligations_at(deed_typeck::Tier::Proven), 1);
}

#[test]
fn a_value_nothing_is_known_about_gets_no_invented_number() {
    // A number would be an invention rather than a finding here, and this
    // diagnostic is read by somebody deciding whether they have a bug.
    let (sources, checked) = check(
        "\
         fn f(n: Int) -> Result<Positive, String> {\n    ok(n)\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("cannot prove this satisfies `Positive`"),
        "{text}"
    );
    assert!(!text.contains("when this is"), "{text}");
}
