# Decision: red-black tree vs list for keyed lookup

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

Issue #616 asked for the measurement that #576 deferred: run the same benchmark
over both `std/map` (red-black tree, `crates/deed-driver/examples/interpreting.rs`)
and `std/table` (list of entries) at the same key counts and decide with the number.

`std/table` documents itself as correct for a handful of keys ("a table of a thousand
keys does a thousand comparisons") and defers the question of when a tree would be
worth having to measurement. `std/map` now exists. This is the measurement.

The benchmark runs 50,000 operations per row, takes the best of five rounds, and
reports the per-operation cost. Source is `crates/deed-driver/examples/interpreting.rs`;
run it with `cargo run -p deed-driver --example interpreting --release`.

Machine: Linux CI runner, single-threaded, interpreter engine only.

## Measured results

### Lookup (get a key that is present, worst case for the list)

```
keys       table/lookup  map/lookup    map faster
0          693ns         677ns         similar
16         3454ns        5252ns        map 1.52x slower
64         11680ns       10501ns       map 1.11x faster
256        43675ns       11542ns       map 3.78x faster
1024       168479ns      13116ns       map 12.84x faster
```

Growth rate check (1024 vs 16 keys, 64x more keys):
- table: 168479 / 3454 = 48.8x (expected ~64x for O(N), consistent)
- map:   13116 / 5252  =  2.5x (expected log2(1024)/log2(16) = 2.5x, exact)

### Insert (key not already present)

The insert benchmark runs `insert(base, "new", ...)` without calling `size` on the
result, so the map pays only O(log N) per iteration. The table benchmark retains its
`length(set(...))` form, which pays O(N) for the set and O(1) for the length.

```
keys       table/insert  map/insert    map faster
0          876ns         2213ns        map 2.53x slower
16         3807ns        26887ns       map 7.06x slower
64         11855ns       40068ns       map 3.38x slower
256        45210ns       47121ns       nearly equal (map 1.04x slower)
1024       180405ns      54607ns       map 3.30x faster
```

Growth rate check (1024 vs 16 keys):
- table: 180405 / 3807 = 47.4x (O(N), consistent)
- map:   54607 / 26887 =  2.0x (O(log N), consistent)

## Decision

The tree is the right default for programs with more than roughly 100 keys for
lookup, or more than roughly 250 keys for insert. Below those sizes the list's
lower constant factor wins. Above them the tree's logarithmic growth wins, and
the gap widens: at 1024 keys lookup is 12.8x faster and insert is 3.3x faster.

Outcome two from #616 (the tree wins asymptotically but loses for small N due to
constant factor) partially describes the result, but the crossover is at 64 and 256
keys, not at "thousands." Real programs that count by key, classify events, or build
indexes typically have more than 64 distinct keys once past a toy example.

`std/table` remains in the library. A table of four keys (like the log-level counter
in `examples/logs.deed`) pays less with the list, and a file that imports `std/table`
continues to work. The question of rewriting `std/table` on top of `std/map` is not
reopened by this measurement, because the two modules serve different size ranges and
making `std/table` a thin wrapper would slow down the small-N case it is designed for.

## What would change this answer

The backend. The map's fixed overhead per insert at small N (26887ns at 16 keys vs
3807ns for the table) is dominated by the recursive function calls and pattern
matches the interpreter pays per tree level. A compiled backend reduces per-call
cost by roughly the ratio in the first table of `interpreting.rs` (walking a turn in
the interpreter vs what compiled code would cost), which would lower the tree's
constant factor and shift the crossover toward smaller N. The measurement above is
interpreter-only and is the right number for programs run under `deed run`. Once the
backend is ready a second measurement is needed; the decision may move but the
direction will not reverse: the tree's growth rate is algorithmic, not a property of
the implementation.

## Drawbacks (required)

The tree has higher constant factor than the list for small N. Programs with fewer
than ~64 keys pay more for lookup and more for insert. Existing code using `std/table`
for small key sets should stay on `std/table`.

The tree requires a comparator argument on every call. A program that builds a map
with one comparator and queries it with another gets silently wrong results; there is
no type-level enforcement. This is documented in `std/map` and in `examples/tree.deed`.

## Rejected Ideas (required)

- **Hash map now**: a hash map would win outright at all sizes. It is not writable in
  Deed today (needs a contiguous indexed representation, copy-free update, and
  structural hashing). These requirements are documented in `design/hash-map-requirements.md`.
  The tree covers the realistic size range in the meantime and requires none of them.

- **Rewrite std/table on top of std/map**: would slow down the small-N case. The two
  modules serve different size ranges. The list is the right substrate for a handful
  of keys; the tree is the right substrate for hundreds or more.

- **Remove std/table**: existing code uses it, the small-N case is legitimately faster,
  and the module carries its own measurement comment updated by #614. Removing it would
  break programs that do not need the tree.

## Open Questions (required)

- The backend answer: does compiled code change the crossover point, and if so where?
  This requires a second benchmark once `deed compile` can compile `std/map`. The
  structural ratchet in `crates/deed-driver/tests/map_scaling.rs` does not depend on
  the backend; it validates tree correctness for both.

- String-key cost: the benchmark uses `cmp_string`, which calls into a built-in string
  comparison. Integer keys via `cmp_int` would be faster and might shift the crossover.
  The logs example uses string keys, so `cmp_string` is the representative case.

## References

- deed-lang/deed#576 (original question: what decides the table vs tree question)
- deed-lang/deed#613 (tree prototype: confirmed comparator-passed-as-fn is enough)
- deed-lang/deed#614 (`std/table` measurement: ~350ns per key for lookup and insert)
- deed-lang/deed#616 (this measurement: compare both at the same sizes)
- deed-lang/deed#617 (structural hashing decision, needed for a future hash map)
- deed-lang/deed#618 (hash map requirements: three gaps not yet filled)
- `std/map.deed` (the red-black tree implementation)
- `std/table.deed` (the list-based implementation)
- `crates/deed-driver/examples/interpreting.rs` (benchmark source)
- `crates/deed-driver/tests/map_scaling.rs` (structural ratchet)
