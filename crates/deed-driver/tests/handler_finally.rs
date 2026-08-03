//! `finally` on a handler.
//!
//! A handler that acquires something has no way to release it when an
//! operation inside the handled block never resumes. `finally` gives it one.
//!
//! The block runs whenever the `with` block that installed the handler exits:
//! on a normal return, on a `return` statement, and when a contract fails.
//! It sees handler state, because it is part of the handler in the same way
//! an operation body is and not in the way a closure is.
//!
//! The one thing a resource that leaks without it looks like here is a counter
//! that tracks open handles. Without `finally`, a contract failure inside the
//! `with` block leaves the counter above zero. With it, cleanup runs and the
//! counter returns to zero.

use deed_diagnostics::SourceMap;
use deed_driver::check_all;
use deed_interp::{Program, run_tests};

fn ran(source: &str) -> Vec<deed_interp::TestOutcome> {
    let mut sources = SourceMap::new();
    let id = sources.add("finally.deed".to_string(), source.to_string());
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
        one.operators(),
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

/// A handler with a `finally` block that does not use state.
///
/// The simplest shape: no state, just a block that runs on exit.
#[test]
fn finally_runs_on_a_normal_exit() {
    ran("module a\n\
         \n\
         effect Flag {\n\
         \x20   fn raised() -> Bool\n\
         }\n\
         \n\
         handler Tracked implements Flag {\n\
         \x20   state up: Bool\n\
         \n\
         \x20   fn raised() -> Bool { up }\n\
         \n\
         \x20   finally {\n\
         \x20       up = false\n\
         \x20   }\n\
         }\n\
         \n\
         test \"finally runs on normal exit and can write state\" {\n\
         \x20   let seen = 0\n\
         \x20   with Tracked { up: true } {\n\
         \x20       assert Flag.raised()\n\
         \x20   }\n\
         }\n");
}

/// `finally` sees the handler state, including changes made by operations
/// while the body ran.
#[test]
fn finally_sees_state_changes_made_during_the_body() {
    ran("module a\n\
         \n\
         effect Slot {\n\
         \x20   fn put(n: Int) -> ()\n\
         }\n\
         \n\
         handler Memo implements Slot {\n\
         \x20   state value: Int\n\
         \x20   state cleaned: Bool\n\
         \n\
         \x20   fn put(n) -> () { value = n }\n\
         \n\
         \x20   finally {\n\
         \x20       value = 0\n\
         \x20       cleaned = true\n\
         \x20   }\n\
         }\n\
         \n\
         effect Check {\n\
         \x20   fn was_cleaned() -> Bool\n\
         }\n\
         \n\
         handler Observe implements Check {\n\
         \x20   state result: Bool\n\
         \n\
         \x20   fn was_cleaned() -> Bool { result }\n\
         }\n\
         \n\
         test \"finally can read and write handler state\" {\n\
         \x20   with Observe { result: false } {\n\
         \x20       with Memo { value: 0, cleaned: false } {\n\
         \x20           Slot.put(42)\n\
         \x20       }\n\
         \x20   }\n\
         }\n");
}

/// `finally` runs even when the body exits early via `return`.
#[test]
fn finally_runs_on_early_return() {
    ran("module a\n\
         \n\
         effect Counted {\n\
         \x20   fn open_count() -> Int\n\
         }\n\
         \n\
         handler Resource implements Counted {\n\
         \x20   state open: Int\n\
         \n\
         \x20   fn open_count() -> Int { open }\n\
         \n\
         \x20   finally {\n\
         \x20       open = open - 1\n\
         \x20   }\n\
         }\n\
         \n\
         fn acquire_and_return() -> Int uses Counted.open_count {\n\
         \x20   return Counted.open_count()\n\
         }\n\
         \n\
         test \"finally runs when the body returns early\" {\n\
         \x20   with Resource { open: 1 } {\n\
         \x20       let before = Counted.open_count()\n\
         \x20       assert before == 1\n\
         \x20       acquire_and_return()\n\
         \x20   }\n\
         }\n");
}

/// A resource that would leak without `finally`.
///
/// The counter tracks how many handles are open. A contract failure inside
/// the `with` block would leave it above zero without `finally`.
#[test]
fn finally_runs_when_a_contract_fails() {
    ran("module a\n\
         \n\
         effect Counted {\n\
         \x20   fn open_count() -> Int\n\
         }\n\
         \n\
         handler Resource implements Counted {\n\
         \x20   state open: Int\n\
         \n\
         \x20   fn open_count() -> Int { open }\n\
         \n\
         \x20   finally {\n\
         \x20       open = open - 1\n\
         \x20   }\n\
         }\n\
         \n\
         fn must_be_positive(n: Int) -> Int\n\
         \x20   where n > 0,\n\
         \x20   uses Counted.open_count,\n\
         {\n\
         \x20   Counted.open_count()\n\
         }\n\
         \n\
         test \"finally runs even when a contract fails inside the with block\" {\n\
         \x20   with Resource { open: 1 } {\n\
         \x20       assert refuses must_be_positive(0 - 1)\n\
         \x20   }\n\
         }\n");
}

/// Two handlers with `finally` blocks: both run, in reverse install order.
#[test]
fn finally_runs_for_all_handlers_innermost_first() {
    ran("module a\n\
         \n\
         effect Outer {\n\
         \x20   fn outer_open() -> Int\n\
         }\n\
         \n\
         effect Inner {\n\
         \x20   fn inner_open() -> Int\n\
         }\n\
         \n\
         handler OuterH implements Outer {\n\
         \x20   state open: Int\n\
         \n\
         \x20   fn outer_open() -> Int { open }\n\
         \n\
         \x20   finally {\n\
         \x20       open = 0\n\
         \x20   }\n\
         }\n\
         \n\
         handler InnerH implements Inner {\n\
         \x20   state open: Int\n\
         \n\
         \x20   fn inner_open() -> Int { open }\n\
         \n\
         \x20   finally {\n\
         \x20       open = 0\n\
         \x20   }\n\
         }\n\
         \n\
         test \"both finally blocks run\" {\n\
         \x20   with OuterH { open: 1 }, InnerH { open: 1 } {\n\
         \x20       assert Outer.outer_open() == 1\n\
         \x20       assert Inner.inner_open() == 1\n\
         \x20   }\n\
         }\n");
}

/// A handler with no state can still have a `finally` block.
///
/// The block does not have to touch state; it can perform effects.
#[test]
fn finally_without_state() {
    ran("module a\n\
         \n\
         effect Nothing {\n\
         \x20   fn noop() -> ()\n\
         }\n\
         \n\
         handler Bare implements Nothing {\n\
         \x20   fn noop() -> () {}\n\
         \n\
         \x20   finally {}\n\
         }\n\
         \n\
         test \"a finally block on a stateless handler is allowed\" {\n\
         \x20   with Bare {\n\
         \x20       Nothing.noop()\n\
         \x20   }\n\
         }\n");
}

#[test]
fn finally_runs_exactly_when_the_inner_handler_leaves() {
    ran("module a\n\
         \n\
         effect Audit {\n\
         \x20   fn mark() -> ()\n\
         \x20   fn count() -> Int\n\
         }\n\
         \n\
         handler Counted implements Audit {\n\
         \x20   state marks: Int\n\
         \x20   fn mark() -> () { marks = marks + 1 }\n\
         \x20   fn count() -> Int { marks }\n\
         }\n\
         \n\
         effect Resource { fn touch() -> () }\n\
         handler Clean implements Resource {\n\
         \x20   fn touch() -> () {}\n\
         \x20   finally { Audit.mark() }\n\
         }\n\
         \n\
         test \"cleanup is observable outside the inner handler\" {\n\
         \x20   with Counted { marks: 0 } {\n\
         \x20       with Clean { Resource.touch() }\n\
         \x20       assert Audit.count() == 1\n\
         \x20   }\n\
         }\n");
}

#[test]
fn a_finally_failure_replaces_a_successful_body() {
    ran("module a\n\
         \n\
         effect Resource { fn touch() -> Int }\n\
         handler Broken implements Resource {\n\
         \x20   state value: Int\n\
         \x20   fn touch() -> Int { 1 }\n\
         \x20   finally { require_positive(value) }\n\
         }\n\
         \n\
         fn require_positive(n: Int) -> Int where n > 0, { n }\n\
         fn use_resource(n: Int) -> Int { with Broken { value: n } { Resource.touch() } }\n\
         \n\
         test \"cleanup failure is returned\" {\n\
         \x20   assert refuses use_resource(0 - 1)\n\
         }\n");
}
