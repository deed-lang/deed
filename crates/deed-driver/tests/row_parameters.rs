//! Where a row variable may be written, now that an effect can take one.
//!
//! `examples/scheduler.deed` answers deed-lang/deed#606: a cooperative
//! round-robin scheduler is writable here, with no continuations and no
//! language machinery. It used to be read as making a second claim it did not
//! make, which is that concurrency in this language is a library, and it was
//! not: the queue's element type had to name every effect a task might
//! perform, so the scheduler had to be copied into every program that wanted
//! one.
//!
//! `std/task` is that scheduler shipping. What made it possible is that an
//! effect may declare row variables, so its operations and the state of any
//! handler implementing it can say `Fn() uses r -> ()`. What keeps it honest
//! is that `r` is filled in at each call that supplies a value for it, which
//! is the same rule a function's row variable has always had.
//!
//! These hold both halves: the positions that opened, and the ones that did
//! not, so that a change to either has to come back through this file.

use deed_diagnostics::SourceMap;
use deed_driver::{check_text, shipped_source};

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

/// The scheduler, as small as it goes: an effect with a row variable and a
/// handler holding a queue of values typed by it.
const SCHEDULER: &str = "module p\n\n\
     effect Task<uses r> {\n\
     \x20   fn fork(step: Fn() uses r -> ()) -> ()\n\
     \x20   fn more() -> Bool\n\
     \x20   fn step() -> ()\n\
     }\n\n\
     handler RoundRobin implements Task {\n\
     \x20   state queue: List<Fn() uses r -> ()>\n\n\
     \x20   fn fork(step) -> () {\n\
     \x20       queue = push(queue, step)\n\
     \x20   }\n\n\
     \x20   fn more() -> Bool {\n\
     \x20       length(queue) > 0\n\
     \x20   }\n\n\
     \x20   fn step() -> ()\n\
     \x20     uses\n\
     \x20       r,\n\
     \x20   {\n\
     \x20       for held in queue with done = () {\n\
     \x20           held()\n\
     \x20       }\n\
     \x20   }\n\
     }\n";

/// An effect takes row variables, and a handler's state may hold values typed
/// by one.
///
/// This is the wall that came down. Both halves of it are here rather than in
/// two tests, because a queue of tasks needs both and neither is worth
/// anything on its own.
#[test]
fn an_effect_is_generic_in_the_row_its_tasks_perform() {
    assert_eq!(
        refused(SCHEDULER),
        Vec::<String>::new(),
        "the scheduler shape no longer checks, so `std/task` cannot be written \
         and the paragraph in examples/scheduler.deed is out of date"
    );
}

/// A type parameter on an effect is still refused.
///
/// A row variable costs nothing to carry because it is erased. A type
/// parameter would have to reach the handler, which is a value with state and
/// one installation, and what `with Q { items: [] }` means when two different
/// `T`s go through it is not decided. See `DEED2024`.
#[test]
fn an_effect_does_not_take_a_type_parameter() {
    let codes = refused(
        "module p\n\n\
         effect Queue<T> {\n\
         \x20   fn add(item: T) -> ()\n\
         }\n",
    );

    assert!(
        codes.contains(&"DEED2024".to_string()),
        "an effect took a type parameter: {codes:?}"
    );
}

/// A row variable in an operation's return type is still refused.
///
/// The rule did not change, only who it applies to. A caller reading
/// `fn next() -> Fn() uses r -> ()` is handed a value performing something it
/// has no name for, so there is nothing it could declare. See `DEED5008`.
#[test]
fn an_operation_cannot_hand_a_row_variable_back() {
    let codes = refused(
        "module p\n\n\
         effect Task<uses r> {\n\
         \x20   fn next() -> Fn() uses r -> ()\n\
         }\n",
    );

    assert!(
        codes.contains(&"DEED5008".to_string()),
        "an effect operation returned a row variable: {codes:?}"
    );
}

/// And one buried inside a parameter that is not itself a function type.
///
/// What fills the variable is the row of the value passed at that parameter,
/// read off its type. A list of functions is not a function, so nothing at the
/// call reads it and the effects would reach the handler uncharged.
#[test]
fn a_row_variable_buried_in_a_parameter_is_still_refused() {
    let codes = refused(
        "module p\n\n\
         effect Task<uses r> {\n\
         \x20   fn fork_all(steps: List<Fn() uses r -> ()>) -> ()\n\
         }\n",
    );

    assert!(
        codes.contains(&"DEED5008".to_string()),
        "a row variable was buried in a list parameter: {codes:?}"
    );
}

/// What a task performs is charged to whoever forked it.
///
/// The half that makes the rest sound. `r` is not a hole a program can put
/// effects through: forking a task that logs is a program that logs, and the
/// row of the function doing the forking has to say so.
#[test]
fn forking_a_task_charges_the_caller_with_what_the_task_performs() {
    let source = format!(
        "{SCHEDULER}\n\
         effect Log {{\n\
         \x20   fn note(message: String) -> ()\n\
         }}\n\n\
         fn noisy() -> ()\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {{\n\
         \x20   Log.note(\"hello\")\n\
         }}\n\n\
         fn go() -> () {{\n\
         \x20   with RoundRobin {{ queue: [] }} {{\n\
         \x20       Task.fork(noisy)\n\
         \x20       Task.step()\n\
         \x20   }}\n\
         }}\n"
    );

    let codes = refused(&source);
    assert!(
        codes.contains(&"DEED5001".to_string()),
        "a function forked a task that logs and was not charged with logging, \
         so a row variable on an effect is a hole rather than a parameter: \
         {codes:?}"
    );
}

/// The variable a handler reads off a type is the one its own effect
/// declared.
///
/// Two effects here, both taking a variable spelled `r`, and the handler
/// implements the second. What `held()` performs is read off the queue's
/// element type, which reaches the effects pass as the name `r` and nothing
/// else, so which `r` that is has to come from the handler. Pick the other
/// one and `run` performs something its `uses` clause does not name.
#[test]
fn a_handler_reads_the_variable_its_own_effect_declared() {
    let codes = refused(
        "module p\n\n\
         effect Idle<uses r> {\n\
         \x20   fn ping(step: Fn() uses r -> ()) -> ()\n\
         }\n\n\
         effect Task<uses r> {\n\
         \x20   fn fork(step: Fn() uses r -> ()) -> ()\n\
         \x20   fn run() -> ()\n\
         }\n\n\
         handler Runner implements Task {\n\
         \x20   state queue: List<Fn() uses r -> ()>\n\n\
         \x20   fn fork(step) -> () {\n\
         \x20       queue = push(queue, step)\n\
         \x20   }\n\n\
         \x20   fn run() -> ()\n\
         \x20     uses\n\
         \x20       r,\n\
         \x20   {\n\
         \x20       for held in queue with done = () {\n\
         \x20           held()\n\
         \x20       }\n\
         \x20   }\n\
         }\n",
    );

    assert_eq!(
        codes,
        Vec::<String>::new(),
        "the handler read a row variable belonging to the wrong effect: {codes:?}"
    );
}

/// An imported effect's operation is type checked against what the effect
/// declared, row variable and all.
///
/// The surface is what carries an operation's parameter types across, and a
/// row variable in one has to cross as a variable rather than as the name of
/// an effect the far side would go looking for. Without the surface entry
/// there is no type to check against and anything at all goes through.
#[test]
fn an_imported_operation_checks_what_it_was_given() {
    let mut sources = SourceMap::new();
    let library = shipped_source("std/task").expect("std/task ships");
    let program = "module p\n\n\
         use std/task.{Task}\n\n\
         fn go() -> () {\n\
         \x20   Task.fork(3)\n\
         }\n";

    let a = sources.add("probe.deed".to_string(), program.to_string());
    let b = sources.add("std/task.deed".to_string(), library.to_string());
    let codes: Vec<String> = deed_driver::check_all(&sources, &[a, b])
        .iter()
        .filter(|checked| checked.file == a)
        .flat_map(|checked| checked.diagnostics.iter())
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code.to_string())
        .collect();

    assert!(
        codes.contains(&"DEED4001".to_string()),
        "an `Int` went where a task was wanted and nothing said so: {codes:?}"
    );
}

/// An imported handler's state crosses with its row variable still a
/// variable.
///
/// `with RoundRobin { queue: [noisy] }` hands the handler a task at the point
/// it is installed, and what the queue holds is `Fn() uses r -> ()`. If `r`
/// arrives on this side as the name of an effect rather than as a variable,
/// there is nothing a task could be that fits it. An empty queue does not ask
/// the question, which is why this one is not empty.
#[test]
fn an_imported_handler_is_installed_holding_a_task() {
    let mut sources = SourceMap::new();
    let library = shipped_source("std/task").expect("std/task ships");
    let program = "module p\n\n\
         use std/task.{Task, RoundRobin, run_up_to}\n\n\
         effect Log {\n\
         \x20   fn note(message: String) -> ()\n\
         }\n\n\
         fn noisy() -> ()\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {\n\
         \x20   Log.note(\"hello\")\n\
         }\n\n\
         fn go() -> Int\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {\n\
         \x20   with RoundRobin { queue: [noisy] } {\n\
         \x20       run_up_to(1)\n\
         \x20   }\n\
         }\n";

    let a = sources.add("probe.deed".to_string(), program.to_string());
    let b = sources.add("std/task.deed".to_string(), library.to_string());
    let codes: Vec<String> = deed_driver::check_all(&sources, &[a, b])
        .iter()
        .filter(|checked| checked.file == a)
        .flat_map(|checked| checked.diagnostics.iter())
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code.to_string())
        .collect();

    assert_eq!(
        codes,
        Vec::<String>::new(),
        "a task did not fit the queue of an imported handler: {codes:?}"
    );
}

/// And a task written into the installation is charged to whoever wrote it
/// there.
///
/// The state is the one way into a handler that does not go through an
/// operation, so it needs the same rule: the task runs, and the function that
/// chose it is the one that answers for what it performs.
#[test]
fn seeding_a_queue_charges_the_caller_too() {
    let mut sources = SourceMap::new();
    let library = shipped_source("std/task").expect("std/task ships");
    let program = "module p\n\n\
         use std/task.{Task, RoundRobin, run_up_to}\n\n\
         effect Log {\n\
         \x20   fn note(message: String) -> ()\n\
         }\n\n\
         fn noisy() -> ()\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {\n\
         \x20   Log.note(\"hello\")\n\
         }\n\n\
         fn go() -> Int {\n\
         \x20   with RoundRobin { queue: [noisy] } {\n\
         \x20       run_up_to(1)\n\
         \x20   }\n\
         }\n";

    let a = sources.add("probe.deed".to_string(), program.to_string());
    let b = sources.add("std/task.deed".to_string(), library.to_string());
    let codes: Vec<String> = deed_driver::check_all(&sources, &[a, b])
        .iter()
        .filter(|checked| checked.file == a)
        .flat_map(|checked| checked.diagnostics.iter())
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code.to_string())
        .collect();

    assert!(
        codes.contains(&"DEED5001".to_string()),
        "a task went into the queue at the installation and nobody was charged \
         with what it performs: {codes:?}"
    );
}

/// A handler for an *imported* parameterised effect cannot name its row
/// variable.
///
/// The row variable travels as a position, which is what a call needs and not
/// what a declaration needs. So `std/task` ships its own handler and a program
/// installs that one; writing a second handler for `Task` in another module is
/// not possible yet. Held here because it is the honest edge of what landed,
/// and because whoever removes it should remove this test in the same change.
#[test]
fn a_handler_elsewhere_cannot_name_an_imported_effects_row_variable() {
    let mut sources = SourceMap::new();
    let library = shipped_source("std/task").expect("std/task ships");
    let program = "module p\n\n\
         use std/task.{Task}\n\n\
         handler Mine implements Task {\n\
         \x20   state queue: List<Fn() uses r -> ()>\n\n\
         \x20   fn fork(step) -> () {\n\
         \x20       queue = push(queue, step)\n\
         \x20   }\n\n\
         \x20   fn more() -> Bool {\n\
         \x20       length(queue) > 0\n\
         \x20   }\n\n\
         \x20   fn step() -> () {\n\
         \x20       ()\n\
         \x20   }\n\
         }\n";

    let a = sources.add("probe.deed".to_string(), program.to_string());
    let b = sources.add("std/task.deed".to_string(), library.to_string());
    let checks = deed_driver::check_all(&sources, &[a, b]);
    let codes: Vec<String> = checks
        .iter()
        .filter(|checked| checked.file == a)
        .flat_map(|checked| checked.diagnostics.iter())
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code.to_string())
        .collect();

    assert!(
        codes.contains(&"DEED3001".to_string()),
        "a handler in another module named `r` and it resolved, so the row \
         variable of an imported effect now crosses the boundary: {codes:?}"
    );
}

/// The scheduler that was written by hand is kept, and it still names an
/// effect that has nothing to do with scheduling.
///
/// That line is the before picture. `Log.note` is in the queue element type
/// because those particular tasks log, and a library could not have written
/// it. Keeping the file and keeping the line is what makes the difference
/// readable next to `examples/tasks.deed`.
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
        "the queue element type no longer names `Log.note`, so the file that \
         shows what a concrete row costs has stopped showing it: {queue}"
    );

    assert!(
        source.contains("That is what `effect Task<uses r>` is"),
        "the paragraph these tests hold has moved or been rewritten"
    );
}
