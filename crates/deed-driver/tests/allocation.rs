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

/// A walk that only pushes allocates what its answer is worth.
///
/// Both rows have the list they walk subtracted off, so what is left is the
/// structure being built: written out, that is the answer and nothing else,
/// and folded it is now the same, because the walk builds one list rather
/// than one a turn. It used to be the answer once per element, 129 times over
/// at this length. See
/// `design/decisions/2026-08-04-a-walk-that-only-pushes.md`.
#[test]
fn folding_a_list_allocates_what_the_answer_is_worth() {
    let base = allocated(&walked(LENGTH)).expect("the walk alone should run");
    let written = allocated(&literal(LENGTH)).expect("a literal should run") - base;
    let built = allocated(&folded(LENGTH)).expect("a fold should run") - base;

    assert!(
        written > 0,
        "writing the list out should allocate something, and it allocated {written} bytes"
    );
    assert_eq!(
        built, written,
        "folding a list of {LENGTH} allocated {built} bytes against {written} written out"
    );
}

/// A walk that does anything else with its accumulator still copies.
///
/// The rule is narrow on purpose, and this is the half of it that would go
/// quiet first: if the shape test stopped ruling anything out, everything
/// would take the fast path and the answers would be wrong rather than slow.
/// `std/list`'s `intersperse` is the shape that found this, by pushing twice
/// in a turn, so a walk that grows by more than one is what this asks about.
#[test]
fn a_walk_that_grows_by_more_than_one_still_copies() {
    // Shorter than the rest, because a walk that copies at the length they
    // use runs out of memory, which is the whole reason for the change this
    // is the other half of.
    const SHORT: usize = 64;

    let base = allocated(&walked(SHORT)).expect("the walk alone should run");
    let written = allocated(&literal(SHORT)).expect("a literal should run") - base;
    let twice_a_turn = allocated(&format!(
        "module bench

test \"walked\" {{
    let source = repeat(0, {SHORT})
    let built = for n in source with out = [] {{ push(push(out, n), n) }}
    assert length(built) == length(source) * 2
}}
"
    ))
    .expect("a fold that pushes twice should run")
        - base;

    assert!(
        twice_a_turn > written * 4,
        "a walk that pushes twice a turn allocated {twice_a_turn} bytes against \
         {written} for the answer, so it took a path that was not written for it"
    );
}

/// Nothing gives a finished list back, so two of them cost two.
///
/// This is the sentence the rest of the reasoning rests on, and the walk
/// building one list rather than one a turn does not change it: a compiled
/// program that builds a structure twice allocates twice, rather than reusing
/// what the first one stopped needing. What is subtracted is the list they
/// both walk, which is built once and shared.
#[test]
fn building_the_same_thing_twice_allocates_twice() {
    let base = allocated(&walked(64)).expect("the walk alone should run");
    let once = allocated(&folded(64)).expect("a fold should run") - base;
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
    .expect("two folds should run")
        - base;

    assert_eq!(
        twice,
        once * 2,
        "a second fold should cost what the first did, and the pair allocated \
         {twice} bytes against {once} for one"
    );
}

/// A walk that carries a record of lists builds each of them once too.
///
/// `partition` and `unzip` and `scan` all carry one, and the accumulator being
/// a record is the only thing that used to put them on the copying path. The
/// record is still built a turn, which is a fixed size and therefore linear,
/// so what this asks is that the part that used to be quadratic no longer is:
/// the two lists together are the answer, and the rest is a record a turn.
/// See `design/decisions/2026-08-04-a-walk-that-pushes-into-a-record.md`.
#[test]
fn a_walk_that_pushes_into_a_record_allocates_along_the_length() {
    let source = |size: usize| {
        format!(
            "module bench

record Parts {{
    kept: List<Int>,
    rest: List<Int>,
}}

test \"walked\" {{
    let source = repeat(0, {size})
    let built = for n at i in source with parts = Parts {{ kept: [], rest: [] }} {{
        if i > 0 {{
            Parts {{ kept: push(parts.kept, i), rest: parts.rest }}
        }} else {{
            Parts {{ kept: parts.kept, rest: push(parts.rest, i) }}
        }}
    }}
    assert length(built.kept) + length(built.rest) == length(source)
}}
"
        )
    };

    let short = 64usize;
    let long = short * 4;
    let near = allocated(&source(short)).expect("a short walk should run")
        - allocated(&walked(short)).expect("the walk alone should run");
    let far = allocated(&source(long)).expect("a longer walk should run")
        - allocated(&walked(long)).expect("the walk alone should run");

    assert!(
        far < near * 8,
        "four times the length allocated {far} bytes against {near}, which is more \
         than a constant a turn, so the record's lists are being copied again"
    );
}
