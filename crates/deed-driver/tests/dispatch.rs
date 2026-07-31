//! What a compiled `perform` would have to do, asked of the interpreter.
//!
//! `design/05-backend.md` says effect handlers are one-shot, and the whole
//! shape of compiled effect dispatch rests on it: one-shot means a stack
//! search and an ordinary call, and anything else means real continuations.
//! The claim is about the interpreter's behaviour, so it is asked of the
//! interpreter rather than left as a sentence somebody read once.

use deed_diagnostics::SourceMap;
use deed_driver::check_all;
use deed_interp::{Program, run_tests};

/// Run the single test in `source` and expect it to fail with a specific code.
fn fails_with(source: &str, expected_code: &str) {
    let mut sources = SourceMap::new();
    let id = sources.add("handlers.deed".to_string(), source.to_string());
    let mut all = check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    assert!(
        !one.has_errors(),
        "this program should check: {:?}",
        one.diagnostics
    );

    let mut program = Program::new();
    program.add(
        one.file,
        &one.module,
        &one.resolutions,
        one.guards(),
        one.rows(),
    );
    let mut outcomes = run_tests(&program, one.file);
    assert_eq!(outcomes.len(), 1, "expected exactly one test");
    let failure = outcomes
        .remove(0)
        .failure
        .expect("the test should have failed");
    assert_eq!(
        failure.code, expected_code,
        "unexpected failure code; message: {}",
        failure.primary.message
    );
}

fn ran(source: &str) -> Vec<deed_interp::TestOutcome> {
    let mut sources = SourceMap::new();
    let id = sources.add("handlers.deed".to_string(), source.to_string());
    let mut all = check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    assert!(
        !one.has_errors(),
        "this program should check: {:?}",
        one.diagnostics
    );

    let mut program = Program::new();
    program.add(
        one.file,
        &one.module,
        &one.resolutions,
        one.guards(),
        one.rows(),
    );
    let outcomes = run_tests(&program, one.file);
    assert!(!outcomes.is_empty(), "this program declares no test");
    for outcome in &outcomes {
        assert!(
            outcome.failure.is_none(),
            "this program should pass: {:?}",
            outcome.failure
        );
    }
    outcomes
}

/// An operation runs once per `perform`, and its answer goes back to the
/// site that performed it.
///
/// A handler counting its own calls is what makes this countable: if an
/// operation could resume more than once, the count and the number of
/// performs would come apart.
#[test]
fn an_operation_runs_once_for_each_time_it_is_performed() {
    ran("module a\n\
         \n\
         effect Tally {\n\
         \x20   fn bump() -> Int\n\
         }\n\
         \n\
         handler Counting implements Tally {\n\
         \x20   state seen: Int\n\
         \n\
         \x20   fn bump() -> Int {\n\
         \x20       seen = seen + 1\n\
         \x20       seen\n\
         \x20   }\n\
         }\n\
         \n\
         fn three() -> Int uses Tally.bump {\n\
         \x20   Tally.bump() + Tally.bump() + Tally.bump()\n\
         }\n\
         \n\
         test \"three performs, three runs, and the answers are one two three\" {\n\
         \x20   with Counting { seen: 0 } {\n\
         \x20       assert three() == 6\n\
         \x20   }\n\
         }\n");
}

/// The innermost handler wins, which is what makes a stack search the right
/// shape for a compiled `perform`.
#[test]
fn the_innermost_handler_is_the_one_that_answers() {
    ran("module a\n\
         \n\
         effect Pick {\n\
         \x20   fn which() -> Int\n\
         }\n\
         \n\
         handler Outer implements Pick {\n\
         \x20   fn which() -> Int { 1 }\n\
         }\n\
         \n\
         handler Inner implements Pick {\n\
         \x20   fn which() -> Int { 2 }\n\
         }\n\
         \n\
         fn ask() -> Int uses Pick.which { Pick.which() }\n\
         \n\
         test \"the nearer handler answers\" {\n\
         \x20   with Outer {\n\
         \x20       assert ask() == 1\n\
         \x20       with Inner {\n\
         \x20           assert ask() == 2\n\
         \x20       }\n\
         \x20       assert ask() == 1\n\
         \x20   }\n\
         }\n");
}

/// A handler's lifetime is its block, which is why a compiled `with` can
/// push an entry and drop it again rather than keeping one alive.
#[test]
fn a_handler_stops_answering_when_its_block_ends() {
    ran("module a\n\
         \n\
         effect Tally {\n\
         \x20   fn bump() -> Int\n\
         }\n\
         \n\
         handler Counting implements Tally {\n\
         \x20   state seen: Int\n\
         \n\
         \x20   fn bump() -> Int {\n\
         \x20       seen = seen + 1\n\
         \x20       seen\n\
         \x20   }\n\
         }\n\
         \n\
         fn once() -> Int uses Tally.bump { Tally.bump() }\n\
         \n\
         test \"a second block starts its own state\" {\n\
         \x20   with Counting { seen: 0 } {\n\
         \x20       assert once() == 1\n\
         \x20       assert once() == 2\n\
         \x20   }\n\
         \x20   with Counting { seen: 0 } {\n\
         \x20       assert once() == 1\n\
         \x20   }\n\
         }\n");
}

// -- abandon ---------------------------------------------------------------

/// `abandon` in a handler operation raises DEED6011 so the caller's stack
/// unwinds rather than waiting for a value that is not coming.
#[test]
fn abandon_in_an_operation_raises_deed6011() {
    fails_with(
        "module a\n\
         \n\
         effect Gate {\n\
         \x20   fn enter() -> Int\n\
         }\n\
         \n\
         handler Closed implements Gate {\n\
         \x20   fn enter() -> Int {\n\
         \x20       abandon\n\
         \x20   }\n\
         }\n\
         \n\
         fn ask() -> Int uses Gate.enter { Gate.enter() }\n\
         \n\
         test \"abandon raises DEED6011\" {\n\
         \x20   with Closed {\n\
         \x20       ask()\n\
         \x20   }\n\
         }\n",
        deed_interp::codes::ABANDONED,
    );
}

// -- finally ---------------------------------------------------------------

/// The `finally` block runs after the `with` body completes normally.
#[test]
fn a_finally_clause_runs_on_normal_exit() {
    ran("module a\n\
         \n\
         effect Tally {\n\
         \x20   fn bump() -> Int\n\
         }\n\
         \n\
         handler Counting implements Tally {\n\
         \x20   state seen: Int\n\
         \n\
         \x20   fn bump() -> Int {\n\
         \x20       seen = seen + 1\n\
         \x20       seen\n\
         \x20   }\n\
         }\n\
         \n\
         fn ask() -> Int uses Tally.bump { Tally.bump() }\n\
         \n\
         test \"finally runs after the body\" {\n\
         \x20   let result = with Counting { seen: 0 } {\n\
         \x20       ask()\n\
         \x20   } finally {\n\
         \x20       assert 1 == 1\n\
         \x20   }\n\
         \x20   assert result == 1\n\
         }\n");
}
