//! What a compiled program does with memory while it runs.
//!
//! `agreement.rs` compares answers, which does not see allocator behaviour.
//! These tests run one allocating loop and check what happens to linear memory.

use deed_codegen::{Trap, Value, call_measured, compile};
use deed_diagnostics::SourceMap;
use deed_driver::check_all;

/// Checks one source file and returns the checked result.
fn checked(source: &str) -> deed_driver::Checked {
    let mut sources = SourceMap::new();
    let id = sources.add("memory.deed".to_string(), source.to_string());
    let mut all = check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    assert!(
        !one.has_errors(),
        "this program should check: {:?}",
        one.diagnostics
    );
    one
}

/// Compiles the source and runs `answer`.
fn measured(source: &str) -> Result<deed_codegen::Outcome, Trap> {
    let one = checked(source);
    let lowered = deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
    let module = compile(&lowered).expect("this compiles");
    call_measured(&module, "answer", &[])
}

/// A loop that allocates both a record and a list on each turn.
fn allocating_loop(turns: usize) -> String {
    let list = std::iter::repeat_n("1", turns)
        .collect::<Vec<_>>()
        .join(", ");
    let payload = (1..=16)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "module a\n\n\
         record Cell {{\n\
         \x20   payload: List<Int>,\n\
         \x20   total: Int,\n\
         }}\n\n\
         fn answer() -> Int {{\n\
         \x20   let done = for n in [{list}] with cell = Cell {{ payload: [], total: 0 }} {{\n\
         \x20       Cell {{ payload: [{payload}], total: cell.total + n }}\n\
         \x20   }}\n\
         \x20   done.total\n\
         }}\n"
    )
}

#[test]
fn allocation_in_a_loop_grows_with_the_number_of_turns() {
    let small = measured(&allocating_loop(100)).expect("this should run");
    let large = measured(&allocating_loop(200)).expect("this should run");

    assert_eq!(small.value, Some(Value::I64(100)));
    assert_eq!(large.value, Some(Value::I64(200)));
    assert!(
        large.allocated >= small.allocated + 100 * 100,
        "100 more turns should allocate much more memory: {} vs {} bytes",
        large.allocated,
        small.allocated
    );
}

/// What used to stop this program was the sixteen pages a module starts
/// with. It grows now, so the same loop runs and allocates past them, and
/// what is left to stop a program is the host rather than the module.
///
/// The half worth keeping from the test this replaces: the growth is real
/// and monotonic, so a program allocating a hundred times more still reaches
/// a hundred times further. Nothing is given back, which is the shape
/// `design/decisions/2026-07-31-compiled-memory-reclamation.md` is about and
/// which growing did not change.
#[test]
fn the_memory_grows_past_the_pages_a_module_starts_with() {
    let pages = 16 * 64 * 1024;
    let far = measured(&allocating_loop(10_000)).expect("this runs now that the memory grows");

    assert_eq!(far.value, Some(Value::I64(10_000)));
    assert!(
        far.allocated > pages,
        "this should be past the {pages} bytes a module starts with, and it allocated {}",
        far.allocated
    );
}

/// And a host that will not grow it any further stops the program rather
/// than letting it write somewhere else.
///
/// The source is small and the allocation is not: a list written out with
/// ten million entries in it would be a test of the parser.
#[test]
fn a_host_that_stops_growing_the_memory_stops_the_program() {
    let source = "module bench\n\n\
         fn answer() -> Int {\n\
         \x20   let built = for n at i in repeat(0, 100000000) with out = [] {\n\
         \x20       push(out, i)\n\
         \x20   }\n\
         \x20   length(built)\n\
         }\n";

    let trap = measured(source).expect_err("nothing has this much room");
    assert!(
        matches!(trap, Trap::Unreachable | Trap::OutOfBounds | Trap::TooLong),
        "it should stop rather than answer: {trap:?}"
    );
}

/// A `with` block inside a walk, which used to leak a handler frame a turn.
///
/// The state is an `Int` so it lives in a slot rather than in memory, which
/// leaves the frame as the only thing a turn allocates. That is what makes the
/// number below readable: any growth here is frames.
fn handler_loop(turns: usize) -> String {
    let list = std::iter::repeat_n("1", turns)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "module a\n\n\
         effect Tick {{\n\
         \x20   fn note() -> ()\n\
         }}\n\n\
         handler Counting implements Tick {{\n\
         \x20   state seen: Int\n\n\
         \x20   fn note() -> () {{\n\
         \x20       seen = seen + 1\n\
         \x20   }}\n\
         }}\n\n\
         fn once() -> Int {{\n\
         \x20   with Counting {{ seen: 0 }} {{\n\
         \x20       Tick.note()\n\
         \x20       1\n\
         \x20   }}\n\
         }}\n\n\
         fn answer() -> Int {{\n\
         \x20   for n in [{list}] with total = 0 {{\n\
         \x20       total + once()\n\
         \x20   }}\n\
         }}\n"
    )
}

/// The same walk with nothing installed, so the handler's cost can be read
/// off the difference rather than guessed at.
fn plain_loop(turns: usize) -> String {
    let list = std::iter::repeat_n("1", turns)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "module a\n\n\
         fn once() -> Int {{\n\
         \x20   1\n\
         }}\n\n\
         fn answer() -> Int {{\n\
         \x20   for n in [{list}] with total = 0 {{\n\
         \x20       total + once()\n\
         \x20   }}\n\
         }}\n"
    )
}

/// Bytes one more turn of a walk costs, given two lengths of it.
fn a_turn(shape: fn(usize) -> String) -> u64 {
    let small = measured(&shape(100)).expect("this should run");
    let large = measured(&shape(400)).expect("this should run");
    assert_eq!(small.value, Some(Value::I64(100)));
    assert_eq!(large.value, Some(Value::I64(400)));
    (large.allocated - small.allocated) / 300
}

/// A walk over numbers allocates a word a turn, and nothing in it is a value
/// that lives in memory.
///
/// Measured rather than assumed, because it is the baseline the next test
/// subtracts and a wrong baseline would credit the handler with the walk's
/// cost or the other way round. What the word is has not been chased down;
/// what it is not is anything the program named.
#[test]
fn a_walk_allocates_a_word_a_turn() {
    assert_eq!(a_turn(plain_loop), 8);
}

/// Installing a handler costs nothing that is not given back.
///
/// A frame's lifetime is exactly its `with` block, nothing in a program can
/// hold one, and blocks nest, so frames are a stack and are reclaimed like
/// one. The state a handler is installed with is on that stack too, and for
/// the same reason: nothing in a program can hold the state itself either,
/// since `DEED4030` refuses a closure over it, and an operation hands back
/// the value in a field rather than the block holding it.
///
/// So a walk that installs a handler every turn now allocates exactly what
/// the same walk without one does. Values are still not given back and cannot
/// be by this argument, because a block's value outlives the block, which is
/// the whole of what is left in
/// `design/decisions/2026-07-31-compiled-memory-reclamation.md`.
#[test]
fn installing_a_handler_costs_nothing_a_turn() {
    let with_one = a_turn(handler_loop);
    let without = a_turn(plain_loop);
    assert_eq!(
        with_one, without,
        "a turn that installs a handler allocates {with_one} bytes against {without} \
         for the same walk without one, so something the `with` block reserved was \
         not given back"
    );
}

/// And the loop that used to run out of memory now does not.
///
/// The same shape as `the_allocator_eventually_runs_out_of_linear_memory`,
/// with the allocation being a handler frame rather than a record. That one
/// still traps, and this one is the difference reclaiming makes.
#[test]
fn a_walk_that_installs_a_handler_every_turn_does_not_run_out() {
    let outcome = measured(&handler_loop(20_000)).expect("this should not run out of memory");
    assert_eq!(outcome.value, Some(Value::I64(20_000)));
}

/// Nesting is what the bound is against, and exceeding it stops rather than
/// writing a frame over the value heap.
///
/// A thousand-deep nest of `with` blocks is not a program anybody writes, and
/// the point of the check is that the failure is a trap rather than silent
/// corruption.
#[test]
fn a_nest_deeper_than_the_frame_stack_traps_rather_than_corrupting_the_heap() {
    let mut source = String::from(
        "module a\n\n\
         effect Tick {\n\
         \x20   fn note() -> ()\n\
         }\n\n\
         handler Counting implements Tick {\n\
         \x20   state seen: Int\n\n\
         \x20   fn note() -> () {\n\
         \x20       seen = seen + 1\n\
         \x20   }\n\
         }\n\n\
         fn deep(n: Int) -> Int\n\
         \x20 uses\n\
         \x20   Diverge,\n\
         {\n\
         \x20   if n <= 0 {\n\
         \x20       0\n\
         \x20   } else {\n\
         \x20       with Counting { seen: 0 } {\n\
         \x20           deep(n - 1) + 1\n\
         \x20       }\n\
         \x20   }\n\
         }\n\n",
    );
    source.push_str("fn answer() -> Int\n  uses\n    Diverge,\n{\n    deep(100000)\n}\n");

    let trap = measured(&source).expect_err("a nest this deep should stop");
    assert!(
        matches!(trap, Trap::Unreachable | Trap::OutOfBounds),
        "it stopped with {trap:?} rather than stopping cleanly"
    );
}
