//! Structural properties of `std/map` that hold regardless of machine speed.
//!
//! The benchmark in `examples/interpreting.rs` measures actual timing for the
//! tree-vs-list comparison. This file ratchets the structural claims that the
//! timing rests on: that the tree is a sorted keyed structure, that it keeps
//! all keys it was given, and where the two modules cross over.
//!
//! A structural claim is one that a count or an exact equality can decide, not
//! one that depends on how fast the machine is. "Entries come back in sorted
//! order after inserts in reverse order" is a structural claim. "Map lookup
//! takes fewer nanoseconds than table lookup at 1024 keys" is a timing claim
//! and does not belong here.
//!
//! The crossover used to be in the second group and is now in the first, and
//! the reason is a change of unit rather than a change of mind. Compiled code
//! is counted in instructions, and an instruction count is the same number on
//! every machine on every run. So which of the two modules costs less at a
//! given key count is something an exact comparison can decide, and the
//! measurement the decision rests on stops being one nobody would notice going
//! stale.
//!
//! See `design/decisions/2026-07-31-tree-vs-table-decision.md` for the
//! measured numbers and the decision they produced.

use deed_diagnostics::SourceMap;
use deed_driver::{check_all, shipped_source};
use deed_interp::{Program, run_tests};

/// Checks `bench` (which imports `std/map`) and runs a named test inside it.
fn run_map_test(bench: &str, test_name: &str) {
    let map_text = shipped_source("std/map")
        .expect("std/map ships")
        .to_string();
    let list_text = shipped_source("std/list")
        .expect("std/list ships")
        .to_string();

    let mut sources = SourceMap::new();
    let bench_id = sources.add("bench.deed", bench.to_string());
    let map_id = sources.add("<shipped>/std/map.deed", map_text);
    let list_id = sources.add("<shipped>/std/list.deed", list_text);

    let checks = check_all(&sources, &[bench_id, map_id, list_id]);
    for checked in &checks {
        let errors: Vec<_> = checked
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .collect();
        assert!(
            errors.is_empty(),
            "bench should check cleanly:\n{}",
            errors
                .iter()
                .map(|d| deed_diagnostics::render_human(&sources, d))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    let mut program = Program::new();
    for checked in &checks {
        program.add(
            checked.file,
            &checked.module,
            &checked.resolutions,
            checked.guards(),
            checked.rows(),
        );
    }

    let outcomes = run_tests(&program, bench_id);
    let named: Vec<_> = outcomes.iter().filter(|o| o.name == test_name).collect();
    assert!(!named.is_empty(), "no test named {test_name:?} was found");
    for outcome in named {
        if let Some(failure) = &outcome.failure {
            panic!(
                "{test_name:?} failed:\n{}",
                deed_diagnostics::render_human(&sources, failure)
            );
        }
    }
}

/// After N distinct inserts the map holds exactly N entries.
///
/// The balancing rotations in a red-black tree must not drop keys. This test
/// inserts 256 distinct integer keys and confirms `size` equals 256. If any
/// rotation discarded a key, the count would be short.
#[test]
fn map_size_equals_insert_count() {
    run_map_test(
        r#"module bench

use std/map.{insert, size, cmp_int, Empty}

test "size matches distinct inserts" {
    let m = for _x at i in repeat(0, 256) with acc = Empty {
        insert(acc, i, i, cmp_int)
    }
    assert size(m) == 256
}
"#,
        "size matches distinct inserts",
    );
}

/// Entries come back in sorted order after inserts in reverse order.
///
/// A red-black tree's in-order traversal is sorted by construction. Inserting
/// in descending order stresses the rebalancing, because every new key goes to
/// the left and forces rotations. If the tree's sorted property held only for
/// ascending inserts, this would fail.
///
/// The expected list is built independently with the same range in ascending
/// order and compared with `==`, so the test is an exact equality check, not a
/// timing claim.
#[test]
fn map_entries_sorted_after_reverse_inserts() {
    run_map_test(
        r#"module bench

use std/map.{insert, entries, cmp_int, Entry, Empty}

test "entries sorted after reverse inserts" {
    let n = 64
    let m = for _x at i in repeat(0, n) with acc = Empty {
        insert(acc, n - 1 - i, i, cmp_int)
    }
    let got = for e in entries(m) with out = [] { push(out, e.key) }
    let expected = for _x at i in repeat(0, n) with out = [] { push(out, i) }
    assert got == expected
    assert length(got) == n
}
"#,
        "entries sorted after reverse inserts",
    );
}

/// Inserting the same key twice keeps exactly one entry with the latest value.
///
/// The uniqueness invariant: a key appears at most once. Replacing a value
/// must not grow the tree. This is verified by checking size stays at 1 and
/// the value is the most recent one.
#[test]
fn map_replace_preserves_uniqueness() {
    run_map_test(
        r#"module bench

use std/map.{insert, size, get, cmp_string, Empty}

test "replacing a key keeps size at one" {
    let m = insert(insert(insert(Empty, "k", 1, cmp_string), "k", 2, cmp_string), "k", 3, cmp_string)
    assert size(m) == 1
    assert get(m, "k", cmp_string) == ok(3)
}
"#,
        "replacing a key keeps size at one",
    );
}

/// How many keys the two sides of the crossover sit at, and how many inserts
/// each measurement makes.
///
/// Insert rather than lookup because it has the wider margin on both sides:
/// the list is ahead by 1.3x at sixteen keys and behind by 1.9x at sixty-four,
/// so neither half of this test is decided by a handful of instructions.
const FEW: usize = 16;
const MANY: usize = 64;
const TURNS: usize = 20;

/// Enough instructions for the largest of these to finish, and a number a
/// stuck program will reach.
const BUDGET: u64 = 100_000_000;

/// The tree overtakes the list between sixteen keys and sixty-four, compiled.
///
/// Two claims, and each one fails on its own. The list is the cheaper
/// substrate for a handful of keys, which is why `std/table` is still in the
/// library. The tree is the cheaper substrate once there are a few dozen,
/// which is why `std/map` was written. A change that made either module
/// uniformly better than the other would fail one half of this and take the
/// decision with it.
#[test]
fn the_compiled_tree_overtakes_the_compiled_list_between_sixteen_keys_and_sixty_four() {
    let list_with_few = inserting(table_insert, FEW);
    let tree_with_few = inserting(map_insert, FEW);
    let list_with_many = inserting(table_insert, MANY);
    let tree_with_many = inserting(map_insert, MANY);

    assert!(
        list_with_few < tree_with_few,
        "with {FEW} keys the list should still be the cheaper insert, and it took \
         {list_with_few} instructions against the tree's {tree_with_few}"
    );
    assert!(
        tree_with_many < list_with_many,
        "with {MANY} keys the tree should be the cheaper insert, and it took \
         {tree_with_many} instructions against the list's {list_with_many}"
    );
}

/// What one insert costs, in instructions: the walk with the inserts in it
/// minus the same program with none, so building the structure is out of it.
fn inserting(bench: fn(usize, usize) -> String, size: usize) -> u64 {
    let walked = instructions(&bench(size, TURNS));
    let setup = instructions(&bench(size, 0));
    (walked - setup) / TURNS as u64
}

fn table_insert(size: usize, turns: usize) -> String {
    format!(
        "module bench

use std/table.{{set}}

test \"inserting\" {{
    let keys = repeat(0, {size})
    let base = for _k at i in keys with entries = [] {{ set(entries, to_string(i), i) }}
    let turns = {turns}
    let ns = repeat(0, turns)
    let got = for _n in ns with sum = 0 {{ sum + length(set(base, \"new\", 0)) }}
    assert got == turns * {}
}}
",
        size + 1
    )
}

fn map_insert(size: usize, turns: usize) -> String {
    format!(
        "module bench

use std/map.{{insert, cmp_string, Empty}}

test \"inserting\" {{
    let keys = repeat(0, {size})
    let base = for _k at i in keys with m = Empty {{ insert(m, to_string(i), i, cmp_string) }}
    let turns = {turns}
    let ns = repeat(0, turns)
    let got = for _n in ns with sum = 0 {{
        let _m = insert(base, \"new\", 0, cmp_string)
        sum + 1
    }}
    assert got == turns
}}
"
    )
}

/// Compiles `bench` with whatever it imports and counts what running its one
/// test executes.
fn instructions(bench: &str) -> u64 {
    let mut sources = SourceMap::new();
    let mut ids = vec![sources.add("bench.deed", bench.to_string())];
    for module in deed_driver::shipped_for([bench]) {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add("<shipped>", text.to_string()));
    }

    let checks = check_all(&sources, &ids);
    for checked in &checks {
        if let Some(diagnostic) = checked.diagnostics.iter().find(|d| d.is_error()) {
            panic!(
                "the benchmark should check cleanly:\n{}",
                deed_diagnostics::render_human(&sources, diagnostic)
            );
        }
    }

    let alongside: Vec<deed_mir::Alongside<'_>> = checks[1..]
        .iter()
        .map(|checked| deed_mir::Alongside {
            module: &checked.module,
            resolutions: &checked.resolutions,
            types: &checked.types,
        })
        .collect();
    let lowered = deed_mir::lower_with_tests_alongside(
        &checks[0].module,
        &checks[0].resolutions,
        &checks[0].types,
        &alongside,
    )
    .expect("the benchmark should lower");
    let compiled = deed_codegen::compile(&lowered).expect("the benchmark should compile");
    let test = lowered
        .tests
        .first()
        .expect("the benchmark declares a test");

    deed_codegen::call_within(&compiled, &test.body, &[], BUDGET)
        .unwrap_or_else(|trap| panic!("the benchmark should pass in the backend: {trap}"))
        .steps
}
