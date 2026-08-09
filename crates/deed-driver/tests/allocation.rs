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

use deed_codegen::{Trap, call_measured, call_within, compile};
use deed_diagnostics::SourceMap;
use deed_driver::check_all;

/// How long a list the rows build. Big enough that the copies dominate and
/// small enough to stay inside a module's memory.
const LENGTH: usize = 256;

/// Bytes the compiled program allocates running its one test.
fn allocated(source: &str) -> Result<u64, Trap> {
    allocated_within(source, None)
}

/// The same, for a program that runs longer than a test's default budget.
fn allocated_within(source: &str, budget: Option<u64>) -> Result<u64, Trap> {
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
    match budget {
        None => call_measured(&module, &test.body, &[]).map(|outcome| outcome.allocated),
        Some(budget) => {
            call_within(&module, &test.body, &[], budget).map(|outcome| outcome.allocated)
        }
    }
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

/// A program that wants more than the pages it started with gets them.
///
/// The sixteen pages a module declares were a starting point and never a
/// decision, and until the memory grew they were a ceiling: a walk building
/// a list of a hundred thousand stopped with "reached past the end of
/// memory", which says nothing about the program and is not an answer to
/// what the caller asked.
///
/// The number is one nothing could reach before. Sixteen pages is a little
/// over a million bytes, the walk holds two lists of eight-byte elements at
/// once, so a hundred thousand is comfortably past it and two hundred
/// thousand is past it twice.
#[test]
fn a_program_that_outgrows_its_first_pages_keeps_going() {
    let past_the_start = |size: usize| {
        format!(
            "module bench

test \"grown\" {{
    let built = for n at i in repeat(0, {size}) with out = [] {{
        push(out, i)
    }}
    assert length(built) == {size}
}}
"
        )
    };

    let pages = 16 * 64 * 1024;
    // More than a test's default budget, because a hundred thousand turns is
    // more work than any test should do by accident and this one means to.
    let budget = Some(200_000_000);
    let grown = allocated_within(&past_the_start(100_000), budget)
        .expect("a walk past the first pages runs");
    assert!(
        grown > pages,
        "this should be past the {pages} bytes a module starts with, and it allocated {grown}"
    );

    allocated_within(&past_the_start(200_000), budget).expect("and one twice as far still runs");
}

/// The same push, one call away, allocates the answer once per turn again.
///
/// `for` knows its accumulator is unshared, so a walk that pushes onto it
/// builds one list. It knows that about the walk rather than about `push`, and
/// a function taking the list and returning it with one more on the end is the
/// same operation with the walk's knowledge cut off: the callee cannot see that
/// its caller is finished with the argument, and nothing tells it.
///
/// What that costs is measured here rather than argued: the copies come back,
/// and doubling the length more than doubles them, which a constant per call
/// would not. See
/// `design/decisions/2026-08-09-what-a-callee-does-with-its-argument.md`.
#[test]
fn the_same_push_behind_a_call_copies_again() {
    let through = |size: usize| {
        format!(
            "module bench

fn added(list: List<Int>, n: Int) -> List<Int> {{ push(list, n) }}

test \"walked\" {{
    let source = repeat(0, {size})
    let built = for n in source with out = [] {{ added(out, n) }}
    assert length(built) == {size}
}}
"
        )
    };
    let excess = |size: usize| {
        let base = allocated(&walked(size)).expect("the walk alone should run");
        allocated(&through(size)).expect("a walk through a call should run") - base
    };

    let written = allocated(&literal(64)).expect("a literal should run")
        - allocated(&walked(64)).expect("the walk alone should run");
    let near = excess(64);
    assert!(
        near > written * 4,
        "a push behind a call allocated {near} bytes for an answer worth {written}"
    );

    let far = excess(128);
    assert!(
        far > near * 3,
        "twice the length allocated {far} against {near}, which is not the shape \
         a fixed cost per call has"
    );
}
