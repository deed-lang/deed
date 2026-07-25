//! Running programs.
//!
//! Most of these are failures, because a runtime that only works when
//! everything is fine is not worth much, and because the failure messages are
//! the product here as much as the evaluation is.

use vow_diagnostics::{SourceMap, render_human};
use vow_interp::{TestOutcome, codes, run_tests};
use vow_lexer::tokenize;
use vow_parser::parse;
use vow_resolve::{Universe, resolve};

fn run(src: &str) -> (SourceMap, Vec<TestOutcome>) {
    run_in(src, &Universe::new())
}

fn run_in(src: &str, universe: &Universe) -> (SourceMap, Vec<TestOutcome>) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.vow", src);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module, universe);
    assert!(!resolved.has_errors(), "test source should resolve cleanly");

    let outcomes = run_tests(file, &parsed.module, &resolved.resolutions);
    (sources, outcomes)
}

/// A universe holding each of `modules`, parsed from source.
///
/// An import with nothing behind it is an error now, so a test about calling
/// across modules needs a real one on the other side.
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

fn expect_pass(src: &str) {
    let (sources, outcomes) = run(src);
    assert!(!outcomes.is_empty(), "no tests were found");
    for outcome in &outcomes {
        if let Some(failure) = &outcome.failure {
            panic!(
                "`{}` should have passed:\n{}",
                outcome.name,
                render_human(&sources, failure)
            );
        }
    }
}

/// The single failure from a source with exactly one test.
fn expect_failure(src: &str) -> (SourceMap, vow_diagnostics::Diagnostic) {
    expect_failure_in(src, &Universe::new())
}

fn expect_failure_in(src: &str, universe: &Universe) -> (SourceMap, vow_diagnostics::Diagnostic) {
    let (sources, mut outcomes) = run_in(src, universe);
    assert_eq!(outcomes.len(), 1, "expected exactly one test");
    let failure = outcomes
        .remove(0)
        .failure
        .expect("the test should have failed");
    (sources, failure)
}

const COUNTER: &str = "\
type Positive = Int where value > 0

effect Counter {
    fn value() -> Int
    fn bump(by: Positive) -> ()
}

handler InMemory implements Counter {
    state count: Int

    fn value() -> Int { count }
    fn bump(by) -> () { count = count + by }
}

handler Frozen implements Counter {
    state count: Int

    fn value() -> Int { count }
    fn bump(by) -> () { }
}
";

fn counter(rest: &str) -> String {
    format!("module a\n\n{COUNTER}\n{rest}")
}

// -- the runnable example --------------------------------------------------

// -- the runnable examples -------------------------------------------------

/// Both examples run. This is the guard that stops either of them drifting
/// into something that only type checks.
#[test]
fn the_examples_pass_their_own_tests() {
    for name in ["counter.vow", "transfer.vow"] {
        let path = format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"));
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("examples/{name} should exist"));

        let mut sources = SourceMap::new();
        let file = sources.add(format!("examples/{name}"), source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        let resolved = resolve(file, &parsed.module, &Universe::new());
        assert!(!resolved.has_errors(), "examples/{name} should resolve");

        let outcomes = run_tests(file, &parsed.module, &resolved.resolutions);
        assert!(!outcomes.is_empty(), "examples/{name} should have tests");
        for outcome in &outcomes {
            if let Some(failure) = &outcome.failure {
                panic!(
                    "examples/{name}, `{}` should pass:\n{}",
                    outcome.name,
                    render_human(&sources, failure)
                );
            }
        }
    }
}

// -- evaluation ------------------------------------------------------------

#[test]
fn arithmetic_and_calls_work() {
    expect_pass(
        "module a\n\n\
         fn double(n: Int) -> Int { n + n }\n\n\
         fn quad(n: Int) -> Int { double(double(n)) }\n\n\
         test \"nesting\" {\n\
         \x20 assert quad(3) == 12\n\
         \x20 assert 7 % 3 == 1\n\
         \x20 assert !(1 > 2)\n\
         }\n",
    );
}

#[test]
fn return_leaves_the_function_early() {
    expect_pass(
        "module a\n\n\
         fn first_positive(n: Int) -> Int {\n\
         \x20 if n > 0 {\n\
         \x20   return n\n\
         \x20 }\n\
         \x20 0\n\
         }\n\n\
         test \"early return\" {\n\
         \x20 assert first_positive(5) == 5\n\
         \x20 assert first_positive(0 - 5) == 0\n\
         }\n",
    );
}

#[test]
fn matching_a_result_binds_the_right_case() {
    expect_pass(
        "module a\n\n\
         choice Failure { TooBig { limit: Int } }\n\n\
         fn small(n: Int) -> Result<Int, Failure> {\n\
         \x20 if n > 10 {\n\
         \x20   return err(TooBig { limit: 10 })\n\
         \x20 }\n\
         \x20 ok(n)\n\
         }\n\n\
         fn handled(n: Int) -> Int {\n\
         \x20 match small(n) {\n\
         \x20   ok(value) => value,\n\
         \x20   err(TooBig { limit }) => 0 - limit,\n\
         \x20 }\n\
         }\n\n\
         test \"both arms\" {\n\
         \x20 assert handled(3) == 3\n\
         \x20 assert handled(50) == 0 - 10\n\
         }\n",
    );
}

#[test]
fn records_and_variants_evaluate() {
    expect_pass(
        "module a\n\n\
         record Point { x: Int, y: Int }\n\n\
         choice Shape { Dot, Line { length: Int } }\n\n\
         fn length(shape: Shape) -> Int {\n\
         \x20 match shape {\n\
         \x20   Dot => 0,\n\
         \x20   Line { length } => length,\n\
         \x20 }\n\
         }\n\n\
         test \"structures\" {\n\
         \x20 let p = Point { x: 1, y: 2 }\n\
         \x20 assert p.x + p.y == 3\n\
         \x20 assert p == Point { y: 2, x: 1 }\n\
         \x20 assert length(Dot) == 0\n\
         \x20 assert length(Line { length: 9 }) == 9\n\
         }\n",
    );
}

#[test]
fn and_short_circuits_before_touching_the_right_hand_side() {
    // The right hand side can perform effects, so this is behaviour rather
    // than an optimisation. `bumped` would move the counter if it ran.
    expect_pass(&counter(
        "fn bumped() -> Bool\n\
         \x20 uses Counter.bump,\n\
         {\n\
         \x20 Counter.bump(1)\n\
         \x20 true\n\
         }\n\n\
         test \"short circuit\" {\n\
         \x20 with InMemory { count: 0 } {\n\
         \x20   assert !(false && bumped())\n\
         \x20   assert Counter.value() == 0\n\
         \x20 }\n\
         }\n",
    ));
}

// -- effects and handlers --------------------------------------------------

#[test]
fn a_handler_can_change_its_own_state() {
    expect_pass(&counter(
        "test \"bumping\" {\n\
         \x20 with InMemory { count: 10 } {\n\
         \x20   Counter.bump(5)\n\
         \x20   assert Counter.value() == 15\n\
         \x20 }\n\
         }\n",
    ));
}

#[test]
fn the_innermost_handler_wins() {
    expect_pass(&counter(
        "test \"nested with\" {\n\
         \x20 with InMemory { count: 1 } {\n\
         \x20   with InMemory { count: 2 } {\n\
         \x20     assert Counter.value() == 2\n\
         \x20   }\n\
         \x20   assert Counter.value() == 1\n\
         \x20 }\n\
         }\n",
    ));
}

#[test]
fn performing_an_effect_with_no_handler_is_a_clean_error() {
    let (sources, failure) = expect_failure(&counter(
        "test \"no handler\" {\n  assert Counter.value() == 0\n}\n",
    ));
    assert_eq!(failure.code, codes::NO_HANDLER);
    let text = render_human(&sources, &failure);
    assert!(
        text.contains("no handler is installed for `Counter`"),
        "{text}"
    );
    assert!(text.contains("`with` block"), "{text}");
}

#[test]
fn a_handler_missing_an_initial_value_says_which() {
    let (sources, failure) = expect_failure(&counter(
        "handler TwoFields implements Counter {\n\
         \x20 state count: Int\n\
         \x20 state limit: Int\n\n\
         \x20 fn value() -> Int { count }\n\
         \x20 fn bump(by) -> () { count = count + by }\n\
         }\n\n\
         test \"incomplete handler\" {\n\
         \x20 with TwoFields { count: 1 } {\n\
         \x20   assert Counter.value() == 1\n\
         \x20 }\n\
         }\n",
    ));
    assert!(render_human(&sources, &failure).contains("initial value for `limit`"));
}

// -- contracts -------------------------------------------------------------

#[test]
fn a_postcondition_that_does_not_hold_blames_the_function() {
    // The frozen handler accepts writes and ignores them, so `bump_twice`
    // cannot keep its promise no matter what the caller does.
    let (sources, failure) = expect_failure(&counter(
        "fn bump_twice(by: Positive) -> Int\n\
         \x20 uses Counter.bump, Counter.value,\n\
         \x20 ensures ok => Counter.value() == old(Counter.value()) + by + by,\n\
         {\n\
         \x20 Counter.bump(by)\n\
         \x20 Counter.bump(by)\n\
         \x20 Counter.value()\n\
         }\n\n\
         test \"frozen breaks the promise\" {\n\
         \x20 with Frozen { count: 0 } {\n\
         \x20   assert bump_twice(5) == 10\n\
         \x20 }\n\
         }\n",
    ));

    assert_eq!(failure.code, codes::POSTCONDITION_FAILED);
    let text = render_human(&sources, &failure);
    assert!(
        text.contains("`bump_twice` did not keep this promise"),
        "{text}"
    );
    assert!(
        text.contains("a bug in the function, not in the caller"),
        "{text}"
    );
}

#[test]
fn the_same_function_keeps_its_promise_with_a_working_handler() {
    expect_pass(&counter(
        "fn bump_twice(by: Positive) -> Int\n\
         \x20 uses Counter.bump, Counter.value,\n\
         \x20 ensures ok => Counter.value() == old(Counter.value()) + by + by,\n\
         {\n\
         \x20 Counter.bump(by)\n\
         \x20 Counter.bump(by)\n\
         \x20 Counter.value()\n\
         }\n\n\
         test \"in memory keeps it\" {\n\
         \x20 with InMemory { count: 100 } {\n\
         \x20   assert bump_twice(5) == 110\n\
         \x20 }\n\
         }\n",
    ));
}

#[test]
fn a_precondition_that_does_not_hold_blames_the_caller() {
    let (sources, failure) = expect_failure(
        "module a\n\n\
         fn halve(n: Int) -> Int\n\
         \x20 where n > 0,\n\
         { n / 2 }\n\n\
         test \"bad call\" {\n\
         \x20 assert halve(0 - 4) == 0 - 2\n\
         }\n",
    );

    assert_eq!(failure.code, codes::PRECONDITION_FAILED);
    let text = render_human(&sources, &failure);
    assert!(
        text.contains("does not satisfy what `halve` requires"),
        "{text}"
    );
    assert!(text.contains("a bug in the caller"), "{text}");
}

#[test]
fn unchanged_is_true_for_a_function_that_only_reads() {
    expect_pass(&counter(
        "fn peek() -> Int\n\
         \x20 uses Counter.value,\n\
         \x20 ensures ok => unchanged(Counter),\n\
         { Counter.value() }\n\n\
         test \"reading changes nothing\" {\n\
         \x20 with InMemory { count: 7 } {\n\
         \x20   assert peek() == 7\n\
         \x20 }\n\
         }\n",
    ));
}

#[test]
fn unchanged_catches_a_function_that_writes() {
    let (sources, failure) = expect_failure(&counter(
        "fn sneaky() -> Int\n\
         \x20 uses Counter.value, Counter.bump,\n\
         \x20 ensures ok => unchanged(Counter),\n\
         {\n\
         \x20 Counter.bump(1)\n\
         \x20 Counter.value()\n\
         }\n\n\
         test \"writing is not unchanged\" {\n\
         \x20 with InMemory { count: 0 } {\n\
         \x20   assert sneaky() == 1\n\
         \x20 }\n\
         }\n",
    ));

    assert_eq!(failure.code, codes::POSTCONDITION_FAILED);
    assert!(render_human(&sources, &failure).contains("`sneaky` did not keep this promise"));
}

#[test]
fn a_refinement_the_compiler_could_not_prove_is_checked_here() {
    // This is the Guarded tier guarding something. The compiler warns that it
    // cannot prove the call, and the interpreter is where that promise is kept.
    let (sources, failure) = expect_failure(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn take(n: Positive) -> Int { n }\n\n\
         fn indirect(n: Int) -> Int { take(n) }\n\n\
         test \"zero is not positive\" {\n\
         \x20 assert indirect(0) == 0\n\
         }\n",
    );

    assert_eq!(failure.code, codes::REFINEMENT_FAILED);
    let text = render_human(&sources, &failure);
    assert!(text.contains("0 does not satisfy `Positive`"), "{text}");
    assert!(text.contains("could not prove this statically"), "{text}");
}

#[test]
fn a_refinement_that_holds_passes_quietly() {
    expect_pass(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn take(n: Positive) -> Int { n }\n\n\
         fn indirect(n: Int) -> Int { take(n) }\n\n\
         test \"one is positive\" {\n\
         \x20 assert indirect(1) == 1\n\
         }\n",
    );
}

// -- assertions ------------------------------------------------------------

#[test]
fn a_failed_comparison_shows_both_sides() {
    // "assertion failed" on its own sends the reader back to run it by hand,
    // which is the round trip the whole design is about.
    let (sources, failure) = expect_failure(
        "module a\n\nfn double(n: Int) -> Int { n + n }\n\ntest \"wrong\" {\n  assert double(3) == 7\n}\n",
    );

    assert_eq!(failure.code, codes::ASSERTION_FAILED);
    let text = render_human(&sources, &failure);
    assert!(text.contains("left is 6, right is 7"), "{text}");
}

// -- arithmetic ------------------------------------------------------------

#[test]
fn division_by_zero_is_a_diagnostic_not_a_panic() {
    let (sources, failure) = expect_failure(
        "module a\n\nfn divide(a: Int, b: Int) -> Int { a / b }\n\ntest \"zero\" {\n  assert divide(1, 0) == 0\n}\n",
    );
    assert_eq!(failure.code, codes::ARITHMETIC);
    assert!(render_human(&sources, &failure).contains("does not wrap"));
}

#[test]
fn overflow_is_a_diagnostic_not_a_wrap() {
    let (sources, failure) = expect_failure(
        "module a\n\nfn grow(n: Int) -> Int { n * 2 }\n\ntest \"too big\" {\n  assert grow(9223372036854775807) == 0\n}\n",
    );
    assert_eq!(failure.code, codes::ARITHMETIC);
    assert!(render_human(&sources, &failure).contains("overflow"));
}

// -- gaps ------------------------------------------------------------------

#[test]
fn calling_into_another_module_is_not_runnable() {
    // The name resolves, so this is not about a missing module. The
    // interpreter only holds the code of the file it was handed, so a call
    // that leaves it has no body to evaluate.
    let (sources, failure) = expect_failure_in(
        "module a\n\nuse std/result.{ok}\n\ntest \"cannot run\" {\n  assert ok(1) == ok(1)\n}\n",
        &universe_of(&["module std/result\n\nfn ok(n: Int) -> Int { n }\n"]),
    );
    assert_eq!(failure.code, codes::NOT_RUNNABLE);
    let text = render_human(&sources, &failure);
    assert!(text.contains("cannot be loaded"), "{text}");
    assert!(
        text.contains("a gap in the interpreter, not something the language forbids"),
        "{text}"
    );
}

#[test]
fn each_test_starts_from_a_clean_slate() {
    let (_, outcomes) = run(&counter(
        "test \"first\" {\n\
         \x20 with InMemory { count: 0 } {\n\
         \x20   Counter.bump(5)\n\
         \x20   assert Counter.value() == 5\n\
         \x20 }\n\
         }\n\n\
         test \"second\" {\n\
         \x20 with InMemory { count: 0 } {\n\
         \x20   assert Counter.value() == 0\n\
         \x20 }\n\
         }\n",
    ));
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(TestOutcome::passed));
}
