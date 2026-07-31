//! Structural properties of `std/map` that hold regardless of machine speed.
//!
//! The benchmark in `examples/interpreting.rs` measures actual timing for the
//! tree-vs-list comparison. This file ratchets the structural claims that the
//! timing rests on: that the tree is a sorted keyed structure and that it
//! keeps all keys it was given.
//!
//! A structural claim is one that a count or an exact equality can decide, not
//! one that depends on how fast the machine is. "Map lookup is faster than
//! table lookup at 1024 keys" is a timing claim. "Entries come back in sorted
//! order after inserts in reverse order" is a structural claim. Both matter;
//! only the second belongs in a ratchet.
//!
//! See `design/decisions/2026-07-31-tree-vs-table-decision.md` for the
//! measured timing numbers and the decision they produced.

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
