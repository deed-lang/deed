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
