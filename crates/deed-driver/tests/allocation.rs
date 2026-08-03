//! How much of what a compiled program allocates is worth anything.
//!
//! `compiled_memory.rs` holds that allocation grows and that the limit is
//! real. This holds the shape of it, which is the part that decides what to do
//! about it: a compiled program gives nothing back except a handler frame, so
//! what it allocates in total is what its memory reached, and the question is
//! how much of that total is a copy of something that died at the same moment.
//!
//! Bytes rather than seconds, so these are counts a compiled program produces
//! rather than measurements of a machine. Nothing here needs rerunning when
//! the machine changes, which is why the numbers can be held at all.
//!
//! See `design/decisions/2026-07-31-compiled-memory-reclamation.md`.

use deed_codegen::{Trap, call_measured, compile};
use deed_diagnostics::SourceMap;
use deed_driver::check_all;

/// How long a list the rows build. Big enough that the copies dominate and
/// small enough to stay inside a module's memory.
const LENGTH: usize = 256;

/// Bytes the compiled program allocates running its one test.
fn allocated(source: &str) -> Result<u64, Trap> {
    let mut sources = SourceMap::new();
    let id = sources.add("bench.deed".to_string(), source.to_string());
    let mut all = check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    assert!(
        !one.has_errors(),
        "this program should check: {:?}",
        one.diagnostics
    );

    let lowered =
        deed_mir::lower_with_tests(&one.module, &one.resolutions, &one.types).expect("this lowers");
    let module = compile(&lowered).expect("this compiles");
    let test = lowered.tests.first().expect("the program declares a test");
    call_measured(&module, &test.body, &[]).map(|outcome| outcome.allocated)
}

/// The list both rows walk, so it can be taken off both of them.
fn walked(size: usize) -> String {
    format!(
        "module bench

test \"walked\" {{
    let source = repeat(0, {size})
    assert length(source) == {size}
}}
"
    )
}

/// The answer written out, which is what the built structure is worth.
fn literal(size: usize) -> String {
    let written = std::iter::repeat_n("0", size)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "module bench

test \"walked\" {{
    let source = repeat(0, {size})
    let built = [{written}]
    assert length(built) == length(source)
}}
"
    )
}

/// The same answer folded, which copies the list it was handed every turn.
fn folded(size: usize) -> String {
    format!(
        "module bench

test \"walked\" {{
    let source = repeat(0, {size})
    let built = for n in source with out = [] {{ push(out, n) }}
    assert length(built) == length(source)
}}
"
    )
}

/// Building a list by folding allocates it once per element.
///
/// Both rows have the list they walk subtracted off, so what is left is the
/// structure being built: written out, that is the answer and nothing else;
/// folded, it is the answer plus every intermediate copy. Each of those copies
/// is dead the moment the next one is made and nothing else points at it,
/// which is exactly the case a reuse analysis answers and the reason that is
/// the direction rather than a collector.
///
/// Held as a floor rather than a number, because what it is about is the
/// shape. A quadratic against a linear cannot be talked out of, and the floor
/// leaves room for the layout to change without this needing an edit. If it
/// ever fails, something reclaimed or reused, and the decision that says
/// nothing does should be reread.
#[test]
fn folding_a_list_allocates_it_once_per_element() {
    let base = allocated(&walked(LENGTH)).expect("the walk alone should run");
    let written = allocated(&literal(LENGTH)).expect("a literal should run") - base;
    let built = allocated(&folded(LENGTH)).expect("a fold should run") - base;

    assert!(
        written > 0,
        "writing the list out should allocate something, and it allocated {written} bytes"
    );
    let copies = built / written;
    assert!(
        copies >= (LENGTH / 4) as u64,
        "folding a list of {LENGTH} allocated {built} bytes against {written} written out, \
         which is {copies}x rather than the order of {LENGTH}x a copy per turn gives"
    );
}

/// Nothing gives the copies back, so the total is what memory reached.
///
/// This is the sentence the rest of the reasoning rests on, and it is one a
/// test can hold: a compiled program that builds a structure twice allocates
/// twice, rather than reusing what the first one stopped needing.
#[test]
fn building_the_same_thing_twice_allocates_twice() {
    let once = allocated(&folded(64)).expect("a fold should run");
    let twice = allocated(
        "module bench

test \"walked\" {
    let source = repeat(0, 64)
    let first = for n in source with out = [] { push(out, n) }
    let second = for n in source with out = [] { push(out, n) }
    assert length(first) == length(second)
}
",
    )
    .expect("two folds should run");

    assert!(
        twice >= once * 2 - once / 8,
        "a second fold should cost about what the first did, and the pair allocated \
         {twice} bytes against {once} for one"
    );
}
