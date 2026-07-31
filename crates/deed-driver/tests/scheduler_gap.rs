//! Why the scheduler is written in Deed and still cannot ship as one.
//!
//! `examples/scheduler.deed` answers deed-lang/deed#606: a cooperative
//! round-robin scheduler is writable here, with no continuations and no
//! language machinery. That file is usually read as making a second claim it
//! does not make, which is that concurrency in this language is a library.
//!
//! It is not, and the reason is one specific gap rather than anything about
//! concurrency. These hold the gap, so that the paragraph in that file falls
//! down if somebody closes it and does not go back to the prose.

use deed_diagnostics::SourceMap;
use deed_driver::check_text;

fn refused(source: &str) -> Vec<String> {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "probe.deed", source.to_string());
    checked
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code.to_string())
        .collect()
}

/// An effect cannot take a row parameter, so a queue of tasks cannot be
/// generic in what its tasks perform.
///
/// This is the wall. A library scheduler's queue would hold
/// `Fn() uses r -> ()`, and `r` would have to belong to the effect, because
/// the handler's `state` is what holds the queue and a handler implements an
/// effect rather than declaring its own parameters.
#[test]
fn an_effect_cannot_be_generic_in_the_row_its_tasks_perform() {
    let codes = refused(
        "module p\n\n\
         effect Schedule<uses r> {\n\
         \x20   fn fork(task: Fn() uses r -> ()) -> ()\n\
         }\n",
    );

    assert!(
        codes.contains(&"DEED2001".to_string()),
        "an effect took a row parameter, so the scheduler paragraph in \
         examples/scheduler.deed is out of date: {codes:?}"
    );
}

/// And a row variable cannot appear in a handler's state either.
///
/// The other half of the same wall, and a separate rule: `DEED5008` confines a
/// row variable to a function-typed parameter's row and the declaration's own
/// `uses`. A `state` is neither, so even an effect that could be parameterised
/// would leave the handler unable to hold the queue.
#[test]
fn a_row_variable_cannot_reach_a_handlers_state() {
    let codes = refused(
        "module p\n\n\
         effect Schedule {\n\
         \x20   fn fork(task: Fn() -> ()) -> ()\n\
         }\n\n\
         handler RoundRobin implements Schedule {\n\
         \x20   state queue: List<Fn() uses r -> ()>\n\n\
         \x20   fn fork(task) -> () {\n\
         \x20       queue = push(queue, task)\n\
         \x20   }\n\
         }\n",
    );

    assert!(
        !codes.is_empty(),
        "a handler's state held a row variable, so the scheduler paragraph in \
         examples/scheduler.deed is out of date"
    );
}

/// The scheduler that does exist names a concrete row, including one effect
/// that is nothing to do with scheduling.
///
/// `Log.note` is in the queue element type because these tasks log. That is
/// the sentence a reader has to be able to check: it is what makes the file an
/// example rather than a library.
#[test]
fn the_written_scheduler_names_an_effect_that_is_not_about_scheduling() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/scheduler.deed"),
    )
    .expect("the scheduler example should be readable");

    let queue = source
        .lines()
        .find(|line| line.contains("state queue:"))
        .expect("the scheduler holds its queue in handler state");

    assert!(
        queue.contains("Log.note"),
        "the queue element type no longer names `Log.note`, so either the \
         example changed or the row-polymorphism gap closed: {queue}"
    );

    assert!(
        source.contains("It is also not a library"),
        "the paragraph these tests hold has moved or been rewritten"
    );
}
