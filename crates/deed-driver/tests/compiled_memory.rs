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

#[test]
fn the_allocator_eventually_runs_out_of_linear_memory() {
    let trap = measured(&allocating_loop(10_000)).expect_err("this should run out of memory");
    assert_eq!(trap, Trap::OutOfBounds);
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

/// The one thing this backend gives back.
///
/// A frame's lifetime is exactly its `with` block, nothing in a program can
/// hold one, and blocks nest, so frames are a stack and are reclaimed like
/// one. Values are not, and cannot be by the same argument: a block's value
/// outlives the block.
///
/// A turn here still allocates sixteen bytes, and they are not the frame. A
/// one-operation frame is four words, and what is left is the handler's state
/// cell and the unit the operation answers with. The state cell could go the
/// same way for the same reason (`DEED4030` already refuses a closure over
/// handler state, so its lifetime is the block too) and that is written in
/// `design/decisions/2026-07-31-compiled-memory-reclamation.md` as the next
/// step rather than done here.
#[test]
fn a_handler_frame_is_given_back_when_its_block_ends() {
    let small = measured(&handler_loop(100)).expect("this should run");
    let large = measured(&handler_loop(400)).expect("this should run");

    assert_eq!(small.value, Some(Value::I64(100)));
    assert_eq!(large.value, Some(Value::I64(400)));

    let a_turn = (large.allocated - small.allocated) / 300;
    assert_eq!(
        a_turn, 16,
        "a turn allocates {a_turn} bytes; a one-operation frame is 32, so this is \
         either the frame coming back or the rest of a turn changing size"
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
