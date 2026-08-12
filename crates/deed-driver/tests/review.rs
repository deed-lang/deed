//! Review receipts compare what two checked module sets ask a reviewer to trust.

use deed_diagnostics::SourceMap;
use deed_driver::{
    Checked, check_text,
    review::{PolicyRule, ReviewPolicy, ReviewReceipt},
};
use deed_typeck::Tier;

fn checked(sources: &mut SourceMap, name: &str, text: &str) -> Checked {
    let checked = check_text(sources, name, text);
    assert!(
        !checked.has_errors(),
        "fixture should check: {:?}",
        checked
            .diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    checked
}

#[test]
fn new_authority_and_a_weaker_proof_are_receipted() {
    let mut before_sources = SourceMap::new();
    let before = checked(
        &mut before_sources,
        "before.deed",
        "module review/sample\n\n\
         type Positive = Int where value > 0\n\n\
         effect Store {\n\
             fn read() -> Int\n\
             fn write(value: Int) -> ()\n\
         }\n\n\
         fn sync() -> Int uses Store.read, { Store.read() }\n\n\
         fn preserve(value: Positive) -> Positive { value + 1 }\n",
    );

    let mut after_sources = SourceMap::new();
    let after = checked(
        &mut after_sources,
        "after.deed",
        "module review/sample\n\n\
         type Positive = Int where value > 0\n\n\
         effect Store {\n\
             fn read() -> Int\n\
             fn write(value: Int) -> ()\n\
         }\n\n\
         fn sync() -> Int\n\
           uses Store.read, Store.write,\n\
         {\n\
             let value = Store.read()\n\
             Store.write(value)\n\
             value\n\
         }\n\n\
         fn preserve(value: Int) -> Positive { value + 1 }\n",
    );

    let receipt = ReviewReceipt::between(&[before], &[after]);
    assert_eq!(receipt.authority_added.len(), 1, "{receipt:?}");
    assert_eq!(receipt.authority_added[0].module, "review/sample");
    assert_eq!(receipt.authority_added[0].declaration, "sync");
    assert_eq!(receipt.authority_added[0].authority, "Store.write");

    assert_eq!(receipt.tier_regressions.len(), 1, "{receipt:?}");
    let regression = &receipt.tier_regressions[0];
    assert_eq!(regression.declaration, "preserve");
    assert_eq!(regression.subject, "Positive");
    assert_eq!(regression.before, Tier::Proven);
    assert_eq!(regression.after, Tier::Guarded);
    assert_eq!(
        receipt.to_json(),
        "{\"kind\":\"review_receipt\",\"clean\":false,\"authority_added\":[{\"module\":\"review/sample\",\"declaration\":\"sync\",\"authority\":\"Store.write\"}],\"tier_regressions\":[{\"module\":\"review/sample\",\"declaration\":\"preserve\",\"subject\":\"Positive\",\"occurrence\":0,\"before\":\"proven\",\"after\":\"guarded\"}],\"guarded_added\":[]}"
    );
}

#[test]
fn less_authority_and_stronger_proofs_need_no_warning() {
    let mut before_sources = SourceMap::new();
    let before = checked(
        &mut before_sources,
        "before.deed",
        "module review/sample\n\n\
         type Positive = Int where value > 0\n\n\
         effect Store { fn write(value: Int) -> () }\n\n\
         fn sync(value: Int) -> () uses Store.write, { Store.write(value) }\n\n\
         fn preserve(value: Int) -> Positive { value + 1 }\n",
    );

    let mut after_sources = SourceMap::new();
    let after = checked(
        &mut after_sources,
        "after.deed",
        "module review/sample\n\n\
         type Positive = Int where value > 0\n\n\
         effect Store { fn write(value: Int) -> () }\n\n\
         fn sync(value: Int) -> () { () }\n\n\
         fn preserve(value: Positive) -> Positive { value + 1 }\n",
    );

    let receipt = ReviewReceipt::between(&[before], &[after]);
    assert!(receipt.is_clean(), "{receipt:?}");
}

#[test]
fn authority_in_a_new_function_is_new_authority() {
    let mut before_sources = SourceMap::new();
    let before = checked(
        &mut before_sources,
        "before.deed",
        "module review/sample\n\neffect Store { fn write(value: Int) -> () }\n",
    );
    let mut after_sources = SourceMap::new();
    let after = checked(
        &mut after_sources,
        "after.deed",
        "module review/sample\n\n\
         effect Store { fn write(value: Int) -> () }\n\n\
         fn save(value: Int) -> () uses Store.write, { Store.write(value) }\n",
    );

    let receipt = ReviewReceipt::between(&[before], &[after]);
    assert_eq!(receipt.authority_added.len(), 1, "{receipt:?}");
    assert_eq!(receipt.authority_added[0].declaration, "save");
    assert_eq!(receipt.authority_added[0].authority, "Store.write");
}

#[test]
fn narrowing_a_whole_effect_to_one_operation_is_not_new_authority() {
    let mut before_sources = SourceMap::new();
    let before = checked(
        &mut before_sources,
        "before.deed",
        "module review/sample\n\n\
         effect Store { fn write(value: Int) -> () }\n\n\
         fn save(value: Int) -> () uses Store, { Store.write(value) }\n",
    );
    let mut after_sources = SourceMap::new();
    let after = checked(
        &mut after_sources,
        "after.deed",
        "module review/sample\n\n\
         effect Store { fn write(value: Int) -> () }\n\n\
         fn save(value: Int) -> () uses Store.write, { Store.write(value) }\n",
    );

    let receipt = ReviewReceipt::between(&[before], &[after]);
    assert!(receipt.is_clean(), "{receipt:?}");
}

#[test]
fn widening_one_operation_to_the_whole_effect_is_new_authority() {
    let mut before_sources = SourceMap::new();
    let before = checked(
        &mut before_sources,
        "before.deed",
        "module review/sample\n\n\
         effect Store { fn write(value: Int) -> () }\n\n\
         fn save(value: Int) -> () uses Store.write, { Store.write(value) }\n",
    );
    let mut after_sources = SourceMap::new();
    let after = checked(
        &mut after_sources,
        "after.deed",
        "module review/sample\n\n\
         effect Store { fn write(value: Int) -> () }\n\n\
         fn save(value: Int) -> () uses Store, { Store.write(value) }\n",
    );

    let receipt = ReviewReceipt::between(&[before], &[after]);
    assert_eq!(receipt.authority_added.len(), 1, "{receipt:?}");
    assert_eq!(receipt.authority_added[0].authority, "Store");
}

#[test]
fn authority_added_by_an_exported_handler_is_receipted() {
    let mut before_sources = SourceMap::new();
    let before = checked(
        &mut before_sources,
        "before.deed",
        "module review/sample\n\n\
         effect Tally { fn add(value: Int) -> () }\n\n\
         effect Audit { fn note(value: Int) -> () }\n\n\
         handler Summer implements Tally {\n\
             state total: Int\n\n\
             fn add(value) -> () { total = total + value }\n\
         }\n",
    );
    let mut after_sources = SourceMap::new();
    let after = checked(
        &mut after_sources,
        "after.deed",
        "module review/sample\n\n\
         effect Tally { fn add(value: Int) -> () }\n\n\
         effect Audit { fn note(value: Int) -> () }\n\n\
         handler Summer implements Tally {\n\
             state total: Int\n\n\
             fn add(value) -> ()\n\
               uses Audit.note,\n\
             {\n\
                 Audit.note(value)\n\
                 total = total + value\n\
             }\n\
         }\n",
    );

    let receipt = ReviewReceipt::between(&[before], &[after]);
    assert_eq!(receipt.authority_added.len(), 1, "{receipt:?}");
    assert_eq!(receipt.authority_added[0].declaration, "Summer");
    assert_eq!(receipt.authority_added[0].authority, "Audit.note");
}

#[test]
fn a_new_guarded_obligation_has_its_own_policy_gate() {
    let mut before_sources = SourceMap::new();
    let before = checked(
        &mut before_sources,
        "before.deed",
        "module review/sample\n\ntype Positive = Int where value > 0\n",
    );
    let mut after_sources = SourceMap::new();
    let after = checked(
        &mut after_sources,
        "after.deed",
        "module review/sample\n\n\
         type Positive = Int where value > 0\n\n\
         fn accept(value: Int) -> Positive { value }\n",
    );

    let receipt = ReviewReceipt::between(&[before], &[after]);
    assert_eq!(receipt.guarded_added.len(), 1, "{receipt:?}");
    assert_eq!(receipt.guarded_added[0].declaration, "accept");
    assert_eq!(receipt.guarded_added[0].subject, "Positive");

    assert!(receipt.evaluate(ReviewPolicy::default()).passed());
    let verdict = receipt.evaluate(ReviewPolicy {
        deny_new_guarded: true,
        ..ReviewPolicy::default()
    });
    assert!(!verdict.passed());
    assert_eq!(verdict.violations.len(), 1);
    assert_eq!(verdict.violations[0].rule, PolicyRule::NewGuarded);
    assert_eq!(verdict.violations[0].findings, 1);
    assert_eq!(
        receipt.to_json_with_policy(&verdict),
        "{\"kind\":\"review_receipt\",\"clean\":false,\"authority_added\":[],\"tier_regressions\":[],\"guarded_added\":[{\"module\":\"review/sample\",\"declaration\":\"accept\",\"subject\":\"Positive\",\"occurrence\":0}],\"policy\":{\"passed\":false,\"violations\":[{\"rule\":\"deny-new-guarded\",\"findings\":1}]}}"
    );
}

#[test]
fn reordering_the_same_subject_does_not_invent_a_change() {
    let mut before_sources = SourceMap::new();
    let before = checked(
        &mut before_sources,
        "before.deed",
        "module review/sample\n\n\
         type Positive = Int where value > 0\n\n\
         fn needs(value: Positive) -> Int { value }\n\n\
         fn combine(known: Positive, unknown: Int) -> Int {\n\
             let first = needs(known + 1)\n\
             let second = needs(unknown)\n\
             first + second\n\
         }\n",
    );
    let mut after_sources = SourceMap::new();
    let after = checked(
        &mut after_sources,
        "after.deed",
        "module review/sample\n\n\
         type Positive = Int where value > 0\n\n\
         fn needs(value: Positive) -> Int { value }\n\n\
         fn combine(known: Positive, unknown: Int) -> Int {\n\
             let second = needs(unknown)\n\
             let first = needs(known + 1)\n\
             first + second\n\
         }\n",
    );

    let receipt = ReviewReceipt::between(&[before], &[after]);
    assert!(receipt.is_clean(), "{receipt:?}");
}

#[test]
fn adding_guarded_beside_proven_does_not_weaken_the_proven_one() {
    let mut before_sources = SourceMap::new();
    let before = checked(
        &mut before_sources,
        "before.deed",
        "module review/sample\n\n\
         type Positive = Int where value > 0\n\n\
         fn needs(value: Positive) -> Int { value }\n\n\
         fn combine(known: Positive) -> Int { needs(known + 1) }\n",
    );
    let mut after_sources = SourceMap::new();
    let after = checked(
        &mut after_sources,
        "after.deed",
        "module review/sample\n\n\
         type Positive = Int where value > 0\n\n\
         fn needs(value: Positive) -> Int { value }\n\n\
         fn combine(known: Positive, unknown: Int) -> Int {\n\
             let guarded = needs(unknown)\n\
             let proven = needs(known + 1)\n\
             guarded + proven\n\
         }\n",
    );

    let receipt = ReviewReceipt::between(&[before], &[after]);
    assert!(receipt.tier_regressions.is_empty(), "{receipt:?}");
    assert_eq!(receipt.guarded_added.len(), 1, "{receipt:?}");
    assert_eq!(receipt.guarded_added[0].subject, "Positive");
}
