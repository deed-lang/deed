//! Running programs.
//!
//! Most of these are failures, because a runtime that only works when
//! everything is fine is not worth much, and because the failure messages are
//! the product here as much as the evaluation is.
//!
//! These are about behaviour, and they read a message where the message is the
//! behaviour. Whether every message the interpreter can produce is read by
//! something is a different question and is answered in `messages.rs`, which
//! also says which of them are read here rather than there.

use deed_diagnostics::{SourceMap, render_human};
use deed_interp::{
    DeclaredRows, Guards, OperatorCalls, Program, TestOutcome, codes, run_main_profiled, run_tests,
};
use deed_lexer::tokenize;
use deed_parser::parse;
use deed_resolve::{Universe, resolve};

fn run(src: &str) -> (SourceMap, Vec<TestOutcome>) {
    run_in(src, &Universe::new())
}

fn run_in(src: &str, universe: &Universe) -> (SourceMap, Vec<TestOutcome>) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module, universe);
    assert!(!resolved.has_errors(), "test source should resolve cleanly");

    // One module, so a call that leaves it has nowhere to go. Tests that need
    // the other side use `run_together`.
    let mut program = Program::new();
    program.add(
        file,
        &parsed.module,
        &resolved.resolutions,
        Guards::new(),
        DeclaredRows::new(),
        OperatorCalls::new(),
    );
    let outcomes = run_tests(&program, file);
    (sources, outcomes)
}

/// Runs the first of `sources` with the rest of them loaded alongside it.
fn run_together(sources_text: &[&str]) -> (SourceMap, Vec<TestOutcome>) {
    let mut sources = SourceMap::new();
    let files: Vec<_> = sources_text
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("file{index}.deed"), *text))
        .collect();

    let parsed: Vec<_> = files
        .iter()
        .map(|file| {
            let lexed = tokenize(*file, sources.file(*file).text());
            parse(*file, &lexed.tokens)
        })
        .collect();

    let mut universe = Universe::new();
    for entry in &parsed {
        universe.add(&entry.module);
    }

    let resolutions: Vec<_> = files
        .iter()
        .zip(&parsed)
        .map(|(file, entry)| resolve(*file, &entry.module, &universe))
        .collect();

    let mut program = Program::new();
    for ((file, entry), resolved) in files.iter().zip(&parsed).zip(&resolutions) {
        program.add(
            *file,
            &entry.module,
            &resolved.resolutions,
            Guards::new(),
            DeclaredRows::new(),
            OperatorCalls::new(),
        );
    }

    let outcomes = run_tests(&program, files[0]);
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
        let file = sources.add(format!("dep{index}.deed"), *source);
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

#[test]
fn profiling_can_separate_contracts_and_handler_operations() {
    let source = "\
module a

type Positive = Int where value > 0

effect Counter {
    fn bump(by: Int) -> Int
}

handler InMemory implements Counter {
    state count: Int

    fn bump(by) -> Int {
        count = count + by
        count
    }
}

fn needs(value: Positive) -> Int { value }

fn work() -> Int
  uses Counter.bump,
{
  with InMemory { count: 0 } {
    let n = Counter.bump(1)
    needs(n)
  }
}

fn main(sys: System) -> Int
  uses Counter.bump,
{
  work()
  0
}
";

    let mut sources = SourceMap::new();
    let file = sources.add("profiled.deed", source);
    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "should parse cleanly");
    let resolved = resolve(file, &parsed.module, &Universe::new());
    assert!(!resolved.has_errors(), "should resolve cleanly");

    let mut program = Program::new();
    program.add(
        file,
        &parsed.module,
        &resolved.resolutions,
        Guards::new(),
        DeclaredRows::new(),
        OperatorCalls::new(),
    );

    let run = run_main_profiled(&program, file, std::path::Path::new(""), &[])
        .expect("there should be a main");
    assert!(run.result.is_ok(), "the run should succeed");

    let profile = run.profile.expect("profiling should be present");
    assert!(
        !profile.functions.is_empty(),
        "at least one function should be reported"
    );

    let needs = profile
        .functions
        .iter()
        .find(|entry| entry.function == "needs")
        .expect("`needs` should be in the profile");
    assert!(needs.calls > 0);
    assert!(
        needs.contract_checks > 0,
        "`needs` should pay for a contract check"
    );

    let bump = profile
        .functions
        .iter()
        .find(|entry| entry.function == "bump")
        .expect("`bump` should be in the profile");
    assert!(bump.calls > 0);
    assert!(
        bump.handler_calls > 0,
        "`bump` should be counted as a handler operation"
    );
}

/// The single failure from a source with exactly one test.
fn expect_failure(src: &str) -> (SourceMap, deed_diagnostics::Diagnostic) {
    expect_failure_in(src, &Universe::new())
}

fn expect_failure_in(src: &str, universe: &Universe) -> (SourceMap, deed_diagnostics::Diagnostic) {
    let (sources, mut outcomes) = run_in(src, universe);
    assert_eq!(outcomes.len(), 1, "expected exactly one test");
    let failure = outcomes
        .remove(0)
        .failure
        .expect("the test should have failed");
    (sources, failure)
}

fn expect_failure_together(sources_text: &[&str]) -> (SourceMap, deed_diagnostics::Diagnostic) {
    let (sources, mut outcomes) = run_together(sources_text);
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
    for name in ["counter.deed", "transfer.deed"] {
        let path = format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"));
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("examples/{name} should exist"));

        let mut sources = SourceMap::new();
        let file = sources.add(format!("examples/{name}"), source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        let resolved = resolve(file, &parsed.module, &Universe::new());
        assert!(!resolved.has_errors(), "examples/{name} should resolve");

        let mut program = Program::new();
        program.add(
            file,
            &parsed.module,
            &resolved.resolutions,
            Guards::new(),
            DeclaredRows::new(),
            OperatorCalls::new(),
        );
        let outcomes = run_tests(&program, file);
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

// -- closures --------------------------------------------------------------

#[test]
fn a_closure_can_be_called() {
    expect_pass(
        "module a\n\n\
         fn adds(a: Int, b: Int) -> Int {\n\
         \x20 let plus = |x: Int, y: Int| { x + y }\n\
         \x20 plus(a, b)\n\
         }\n\n\
         test \"calling one\" {\n\
         \x20 assert adds(2, 3) == 5\n\
         }\n",
    );
}

#[test]
fn a_closure_sees_the_names_around_it() {
    expect_pass(
        "module a\n\n\
         fn bumped(n: Int) -> Int {\n\
         \x20 let bump = || { n + 1 }\n\
         \x20 bump()\n\
         }\n\n\
         test \"capture\" {\n\
         \x20 assert bumped(41) == 42\n\
         }\n",
    );
}

#[test]
fn a_closure_can_hold_another_closure() {
    expect_pass(
        "module a\n\n\
         fn f(n: Int) -> Int {\n\
         \x20 let outer = |x: Int| {\n\
         \x20   let inner = |y: Int| { y * 2 }\n\
         \x20   inner(x) + n\n\
         \x20 }\n\
         \x20 outer(5)\n\
         }\n\n\
         test \"nesting\" {\n\
         \x20 assert f(1) == 11\n\
         }\n",
    );
}

#[test]
fn a_closure_can_perform_an_effect_its_author_declared() {
    expect_pass(&counter(
        "fn bump_through_a_closure() -> Int\n\
         \x20 uses\n\
         \x20   Counter.bump,\n\
         \x20   Counter.value,\n\
         {\n\
         \x20 let bump = || { Counter.bump(1) }\n\
         \x20 bump()\n\
         \x20 Counter.value()\n\
         }\n\n\
         test \"an effect from inside a closure\" {\n\
         \x20 with InMemory { count: 0 } {\n\
         \x20   assert bump_through_a_closure() == 1\n\
         \x20 }\n\
         }\n",
    ));
}

#[test]
fn two_closures_are_not_the_same_closure() {
    // Equality is identity. Comparing captured frames would call two closures
    // equal when calling them does different things.
    expect_pass(
        "module a\n\n\
         fn f() -> Bool {\n\
         \x20 let one = || { 1 }\n\
         \x20 let same = one\n\
         \x20 same == one\n\
         }\n\n\
         test \"identity\" {\n\
         \x20 assert f()\n\
         }\n",
    );
}

// -- termination -----------------------------------------------------------

#[test]
fn bounded_recursion_still_returns() {
    expect_pass(
        "module a\n\n\
         fn factorial(n: Int) -> Int\n\
         \x20 uses\n\
         \x20   Diverge,\n\
         {\n\
         \x20 if n <= 1 {\n\
         \x20   1\n\
         \x20 } else {\n\
         \x20   n * factorial(n - 1)\n\
         \x20 }\n\
         }\n\n\
         test \"it comes back\" {\n\
         \x20 assert factorial(5) == 120\n\
         }\n",
    );
}

#[test]
fn unbounded_recursion_is_reported_rather_than_fatal() {
    // The point of this one is that it finishes at all. `Diverge` in the row
    // says a function may not return; it does not make one return, and a
    // runner that can be taken down by the program it is running is a runner
    // nobody can point at a file they have not read.
    let (_, failure) = expect_failure(
        "module a\n\n\
         fn forever(n: Int) -> Int\n\
         \x20 uses\n\
         \x20   Diverge,\n\
         {\n\
         \x20 forever(n + 1)\n\
         }\n\n\
         test \"never\" {\n\
         \x20 assert forever(0) == 0\n\
         }\n",
    );
    assert_eq!(failure.code, codes::TOO_DEEP);
}

#[test]
fn a_closure_calling_itself_is_bounded_too() {
    // A closure is called through a different path, so the limit has to be on
    // both or one of them still takes the process down.
    let (_, failure) = expect_failure(
        "module a\n\n\
         fn f() -> Int\n\
         \x20 uses\n\
         \x20   Diverge,\n\
         {\n\
         \x20 go(0)\n\
         }\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses\n\
         \x20   Diverge,\n\
         {\n\
         \x20 let step = |x: Int| { go(x + 1) }\n\
         \x20 step(n)\n\
         }\n\n\
         test \"never\" {\n\
         \x20 assert f() == 0\n\
         }\n",
    );
    assert_eq!(failure.code, codes::TOO_DEEP);
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
    // The code as well as the words. Swapping the constant at this one
    // emission site used to leave the whole workspace green, which is half of
    // a message being held: the wording is read here and nothing at all said
    // which code it arrives under.
    assert_eq!(failure.code, codes::NOT_RUNNABLE);
    assert!(render_human(&sources, &failure).contains("initial value for `limit`"));
}

#[test]
fn a_handler_with_no_state_can_be_installed_either_way() {
    // `with Quiet { }` used to be unparseable, because the rule that tells a
    // handler literal from the block after it looks for `name:` and there is
    // no name in an empty one. Both spellings run now, and they are the same
    // handler.
    expect_pass(
        "module a\n\n\
         effect Counter {\n\
         \x20 fn value() -> Int\n\
         \x20 fn bump(by: Int) -> ()\n\
         }\n\n\
         handler Frozen implements Counter {\n\
         \x20 fn value() -> Int { 0 }\n\
         \x20 fn bump(by) -> () { () }\n\
         }\n\n\
         fn counted() -> Int\n\
         \x20 uses Counter.value,\n\
         {\n\
         \x20 Counter.value()\n\
         }\n\n\
         test \"no braces\" {\n\
         \x20 with Frozen {\n\
         \x20   assert counted() == 0\n\
         \x20 }\n\
         }\n\n\
         test \"empty braces\" {\n\
         \x20 with Frozen { } {\n\
         \x20   assert counted() == 0\n\
         \x20 }\n\
         }\n",
    );
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

/// `unchanged` compares against what entering the call captured, and entering
/// captures for the sake of the `ensures` clauses. Outside a contract there is
/// nothing captured for it to be about, so it is refused, which is the rule
/// `old` has had all along.
///
/// It used to answer, because every call captured whether or not anything
/// could read it, and what it answered from once calls stopped doing that
/// would have been a different call's snapshot.
#[test]
fn unchanged_outside_a_contract_is_refused() {
    let (sources, failure) = expect_failure(&counter(
        "fn peek() -> Bool\n\
         \x20 uses Counter.value,\n\
         {\n\
         \x20 let _seen = Counter.value()\n\
         \x20 unchanged(Counter)\n\
         }\n\n\
         test \"outside a contract\" {\n\
         \x20 with InMemory { count: 7 } {\n\
         \x20   assert peek()\n\
         \x20 }\n\
         }\n",
    ));

    assert_eq!(failure.code, codes::NOT_RUNNABLE);
    assert!(
        render_human(&sources, &failure).contains("`unchanged` outside a contract"),
        "{}",
        render_human(&sources, &failure)
    );
}

// Refinements at runtime live in `deed-driver/tests/guards.rs`. What has to be
// checked is what the type checker gave up on, so a test for it has to run the
// checker, and these tests deliberately do not.

// -- contracts, through a closure ------------------------------------------

/// `result` inside a closure inside a contract.
///
/// The interpreter finds out whether an obligation mentions `result` by
/// walking it, and that walk did not go into a closure body. `deed check`
/// accepted this and running it then said `DEED6006`, whose own note says
/// either the file was not checked or the check has a hole. It was the hole.
///
/// Found by writing the contract the benchmark's `largest` answer was missing:
/// `any(numbers, |n: Int| n == result)` is the natural way to say "the answer
/// came out of the list", and it was the one shape that could not run.
#[test]
fn result_inside_a_closure_in_a_contract_runs() {
    expect_pass(
        "module a\n\n\
         fn holds(f: Fn(Int) -> Bool, n: Int) -> Bool { f(n) }\n\n\
         fn same(n: Int) -> Int\n\
         \x20 ensures ok => holds(|x: Int| x == result, n),\n\
         { n }\n\n\
         test \"through a closure\" {\n\
         \x20 assert same(3) == 3\n\
         }\n",
    );
}

/// And it is enforced rather than merely runnable. A contract that runs and
/// always says yes is worse than one that refuses to run, because nothing
/// announces it.
#[test]
fn result_inside_a_closure_still_catches_a_broken_promise() {
    let (sources, failure) = expect_failure(
        "module a\n\n\
         fn holds(f: Fn(Int) -> Bool, n: Int) -> Bool { f(n) }\n\n\
         fn off_by_one(n: Int) -> Int\n\
         \x20 ensures ok => holds(|x: Int| x == result, n),\n\
         { n + 1 }\n\n\
         test \"the promise is broken\" {\n\
         \x20 assert off_by_one(3) == 4\n\
         }\n",
    );
    assert_eq!(failure.code, codes::POSTCONDITION_FAILED);
    assert!(render_human(&sources, &failure).contains("`off_by_one` did not keep this promise"));
}

/// The same gap, in the same walk, for `old`. Both walkers stopped at a
/// closure body and neither knew about the other, so they are one walk now.
#[test]
fn old_inside_a_closure_in_a_contract_runs() {
    expect_pass(&counter(
        "fn holds(f: Fn(Int) -> Bool, n: Int) -> Bool { f(n) }\n\n\
         fn bump_twice(by: Positive) -> Int\n\
         \x20 uses Counter.bump, Counter.value,\n\
         \x20 ensures ok => holds(|v: Int| v == old(Counter.value()) + by + by, Counter.value()),\n\
         {\n\
         \x20 Counter.bump(by)\n\
         \x20 Counter.bump(by)\n\
         \x20 Counter.value()\n\
         }\n\n\
         test \"old through a closure\" {\n\
         \x20 with InMemory { count: 100 } {\n\
         \x20   assert bump_twice(5) == 110\n\
         \x20 }\n\
         }\n",
    ));
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

// -- strings ---------------------------------------------------------------

#[test]
fn strings_join_and_measure() {
    expect_pass(
        "module a\n\n\
         fn greet(name: String) -> String { \"hello, \" + name }\n\n\
         test \"joining\" {\n\
         \x20 assert greet(\"onat\") == \"hello, onat\"\n\
         \x20 assert \"\" + \"a\" == \"a\"\n\
         \x20 assert length(\"hello\") == 5\n\
         \x20 assert length(\"\") == 0\n\
         }\n",
    );
}

#[test]
fn a_length_counts_characters_and_not_bytes() {
    // Otherwise a refinement written against it means something different
    // depending on which letters happened to turn up.
    expect_pass(
        "module a\n\n\
         test \"characters\" {\n\
         \x20 assert length(\"é\") == 1\n\
         \x20 assert length(\"gün\") == 3\n\
         }\n",
    );
}

#[test]
fn strings_compare_in_order() {
    expect_pass(
        "module a\n\n\
         test \"ordering\" {\n\
         \x20 assert \"a\" < \"b\"\n\
         \x20 assert \"abc\" < \"abd\"\n\
         \x20 assert \"ab\" < \"abc\"\n\
         \x20 assert !(\"b\" <= \"a\")\n\
         }\n",
    );
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

// -- across modules --------------------------------------------------------

#[test]
fn a_call_into_another_module_runs_that_module_s_code() {
    let (sources, outcomes) = run_together(&[
        "module a\n\nuse b.{twice}\n\ntest \"it runs\" {\n  assert twice(21) == 42\n}\n",
        "module b\n\nfn twice(n: Int) -> Int { n + n }\n",
    ]);
    for outcome in &outcomes {
        if let Some(failure) = &outcome.failure {
            panic!("{}", render_human(&sources, failure));
        }
    }
    assert_eq!(outcomes.len(), 1);
}

#[test]
fn a_callee_reads_its_own_names_not_the_caller_s() {
    // Both modules declare `limit`, with different values. The body of
    // `capped` has to see its own, or a call across a boundary would quietly
    // pick up whatever the caller happened to have in scope.
    let (sources, outcomes) = run_together(&[
        "module a\n\nuse b.{capped}\n\nfn limit() -> Int { 1 }\n\ntest \"its own\" {\n  assert capped() == 100\n}\n",
        "module b\n\nfn limit() -> Int { 100 }\n\nfn capped() -> Int { limit() }\n",
    ]);
    for outcome in &outcomes {
        if let Some(failure) = &outcome.failure {
            panic!("{}", render_human(&sources, failure));
        }
    }
    assert_eq!(outcomes.len(), 1);
}

#[test]
fn a_variant_is_the_same_variant_on_both_sides_of_an_import() {
    let (sources, outcomes) = run_together(&[
        "module a\n\nuse b.{Loud, Tone, loudest}\n\ntest \"same variant\" {\n  assert loudest() == Loud\n}\n",
        "module b\n\nchoice Tone {\n  Plain,\n  Loud,\n}\n\nfn loudest() -> Tone { Loud }\n",
    ]);
    for outcome in &outcomes {
        if let Some(failure) = &outcome.failure {
            panic!("{}", render_human(&sources, failure));
        }
    }
    assert_eq!(outcomes.len(), 1);
}

#[test]
fn a_contract_on_an_imported_function_is_still_enforced() {
    // The call crosses a boundary, the precondition does not stop applying.
    let (sources, failure) = expect_failure_together(&[
        "module a\n\nuse b.{halve}\n\ntest \"breaks it\" {\n  assert halve(3) == 1\n}\n",
        "module b\n\nfn halve(n: Int) -> Int\n  where\n    n % 2 == 0,\n{\n  n / 2\n}\n",
    ]);
    assert_eq!(failure.code, codes::PRECONDITION_FAILED);
    let text = render_human(&sources, &failure);
    assert!(text.contains("halve"), "{text}");
}

/// The file, not only the offsets.
///
/// A `Diagnostic` carries one file for the whole of it and a `Span` carries
/// none, so a failure built while the callee's module is current but pointing
/// at the caller's byte offsets underlines whatever happens to sit at those
/// offsets in the wrong file. That is the shape of the worst part of #257, a
/// contract failure landing inside a callee that had declared everything
/// correctly, and it was still live for this one.
///
/// A `Label` carries a file of its own now, so the secondary is no longer a
/// choice between the wrong bytes and nothing.
///
/// The two files are deliberately different lengths, so that reading the
/// caller's offsets out of the callee's text cannot land on the same words by
/// accident.
#[test]
fn a_precondition_that_fails_across_a_module_boundary_underlines_the_call() {
    let (sources, failure) = expect_failure_together(&[
        "module a\n\nuse b.{halve}\n\ntest \"breaks it\" {\n  assert halve(3) == 1\n}\n",
        "module b\n\n// Longer than the file that calls into it, on purpose.\n\n\
         fn halve(n: Int) -> Int\n  where\n    n % 2 == 0,\n{\n  n / 2\n}\n",
    ]);
    assert_eq!(failure.code, codes::PRECONDITION_FAILED);

    let file = sources.file(failure.file);
    let span = failure.primary.span;
    let underlined = &file.text()[span.start as usize..span.end as usize];
    assert_eq!(underlined, "halve(3)", "underlined in `{}`", file.name());

    // And the clause is shown, in the file it is written in. This used to be
    // left off entirely, because a label carried a span and no file and would
    // have been drawn over whatever sat at those offsets in the caller. It
    // says which file it means now, so both halves of the failure are on the
    // screen: what the caller wrote and what it did not satisfy.
    let [clause] = failure.secondary.as_slice() else {
        panic!("{}", render_human(&sources, &failure));
    };
    let clause_file = sources.file(clause.file_or(failure.file));
    assert_ne!(
        clause.file_or(failure.file),
        failure.file,
        "the clause is in the callee and the diagnostic is filed against the caller"
    );
    assert_eq!(
        &clause_file.text()[clause.span.as_range()],
        "n % 2 == 0",
        "underlined in `{}`",
        clause_file.name()
    );

    // And the rendering says so, or a reader watches the caret change files
    // with nothing to tell them it did.
    let text = render_human(&sources, &failure);
    assert!(text.contains(clause_file.name()), "{text}");
}

/// The same rule, for the depth limit.
///
/// The span is the call and the check runs after the callee's module has been
/// made current, so this had the same fault and needed the same fix. Mutual
/// recursion is what makes it visible: with both ends in one file there is
/// nothing to get wrong.
#[test]
fn a_run_that_goes_too_deep_across_a_module_boundary_underlines_a_call() {
    let (sources, failure) = expect_failure_together(&[
        "module a\n\nuse b.{descend}\n\n\
         fn ascend(n: Int) -> Int\n  uses\n    Diverge,\n{\n  descend(n + 1)\n}\n\n\
         test \"deep\" {\n  assert ascend(0) == 0\n}\n",
        "module b\n\n// Longer than the file that calls into it, on purpose.\n\n\
         use a.{ascend}\n\n\
         fn descend(n: Int) -> Int\n  uses\n    Diverge,\n{\n  ascend(n + 1)\n}\n",
    ]);
    assert_eq!(failure.code, codes::TOO_DEEP);

    let file = sources.file(failure.file);
    let span = failure.primary.span;
    let underlined = &file.text()[span.start as usize..span.end as usize];
    assert!(
        underlined == "descend(n + 1)" || underlined == "ascend(n + 1)",
        "underlined `{underlined}` in `{}`",
        file.name()
    );
}

/// The other half of the same rule.
///
/// A postcondition failure is the function's bug, so the primary stays on the
/// clause and the file stays the callee's. What moves is the `called from
/// here` label, and it goes to the caller's file and says so. For a library
/// function called from twenty places that label is the whole diagnosis, and
/// it used to be left off whenever the call was in another module.
#[test]
fn a_postcondition_that_fails_across_a_module_boundary_names_the_call() {
    let (sources, failure) = expect_failure_together(&[
        "module a\n\nuse b.{promise}\n\ntest \"breaks it\" {\n  assert promise(1) == 1\n}\n",
        "module b\n\n// Longer than the file that calls into it, on purpose.\n\n\
         fn promise(n: Int) -> Int\n  ensures\n    ok => result > 100,\n{\n  n\n}\n",
    ]);
    assert_eq!(
        failure.code,
        codes::POSTCONDITION_FAILED,
        "{}",
        render_human(&sources, &failure)
    );

    let file = sources.file(failure.file);
    let span = failure.primary.span;
    let underlined = &file.text()[span.start as usize..span.end as usize];
    assert_eq!(
        underlined,
        "ok => result > 100",
        "underlined in `{}`",
        file.name()
    );

    let [call] = failure.secondary.as_slice() else {
        panic!("{}", render_human(&sources, &failure));
    };
    let call_file = sources.file(call.file_or(failure.file));
    assert_ne!(
        call.file_or(failure.file),
        failure.file,
        "the call is in the caller and the diagnostic is filed against the function"
    );
    assert_eq!(
        &call_file.text()[call.span.as_range()],
        "promise(1)",
        "underlined in `{}`",
        call_file.name()
    );

    let text = render_human(&sources, &failure);
    assert!(text.contains(call_file.name()), "{text}");
}

#[test]
fn a_call_into_a_module_that_was_not_handed_over_says_so() {
    // The name resolves, because the module is in the universe the resolver
    // was given. The interpreter was handed one file, so there is no body.
    let (sources, failure) = expect_failure_in(
        "module a\n\nuse std/result.{ok}\n\ntest \"cannot run\" {\n  assert ok(1) == ok(1)\n}\n",
        &universe_of(&["module std/result\n\nfn ok(n: Int) -> Int { n }\n"]),
    );
    assert_eq!(failure.code, codes::NOT_RUNNABLE);
    let text = render_human(&sources, &failure);
    assert!(text.contains("was not handed to the interpreter"), "{text}");
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
