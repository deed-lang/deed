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

Machine: Linux CI runner, single-threaded, interpreter engine only. The compiled
section added below is in instructions rather than seconds and so does not have a
machine.

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

## Measured results, compiled

Added 2026-08-03, answering the open question this document left. Same programs,
same key counts, through `deed`'s WebAssembly backend rather than the interpreter.

The unit is instructions rather than nanoseconds, and the reason is not that timing
compiled code is hard. What runs the module here is `crates/deed-codegen/src/run.rs`,
an interpreter over the instructions the compiler emits, so its clock is a fact about
that runner. Its instruction count is a fact about the compiled program: it is the
work any engine would have to do, it is the same number on every machine, and a
number kept in a document should not have to be reread when the machine underneath
it changes. Each row is the walk with 200 operations minus the same program with
none, so building the structure is subtracted out and what is left is the operations.

### Lookup (get a key that is present)

```
keys       table/lookup  map/lookup    map faster
0          146           147           the same
16         1533          979           map 1.57x faster
64         5851          1573          map 3.72x faster
256        22613         2204          map 10.26x faster
1024       out of memory 2451          the table does not run
```

Growth rate check (256 vs 16 keys, 16x more keys):
- table: 22613 / 1533 = 14.8x (expected ~16x for O(N), consistent)
- map:   2204 / 979   =  2.3x (expected log2(256)/log2(16) = 2.0x, consistent)

### Insert (key not already present)

```
keys       table/insert  map/insert    map faster
0          187           267           map 1.43x slower
16         1675          2198          map 1.31x slower
64         6139          3196          map 1.92x faster
256        27739         4021          map 6.90x faster
1024       out of memory out of memory neither runs
```

Growth rate check (256 vs 16 keys):
- table: 27739 / 1675 = 16.6x (O(N), consistent)
- map:   4021 / 2198  =  1.8x (O(log N), consistent)

### The crossover moved, in the direction this document predicted

| | interpreted | compiled |
|---|---|---|
| lookup | between 64 and 256 keys | below 16 keys |
| insert | between 256 and 1024 keys | between 16 and 64 keys |

The tree's constant factor was the interpreter's per-call cost, as predicted. Removing
it does not change either growth rate, which is what makes this a compiler question
rather than an algorithm one: the two curves are the same shape and the tree's starts
lower.

### A thousand keys does not run at all

Both 1024-key rows stop with `reached past the end of memory`, and that is not a
benchmark accident. A compiled module gets sixteen pages, one megabyte
(`crates/deed-codegen/src/compile.rs`), and a handler frame is the only thing this
backend reclaims (`crates/deed-codegen/src/layout.rs` says why: a block's value
outlives the block). `std/table`'s `set` copies the list, so building a 1024-key table
allocates on the order of half a million entries. The tree allocates a fresh path per
insert rather than a fresh table, which is why it survives the lookup benchmark and
not the insert one: building the tree is already close to the megabyte, and 200 more
inserts cross it.

So above a few hundred keys the choice between these two modules is not the thing
stopping a compiled program. Value reclamation is.

## Decision

The tree is the right default for programs with more than roughly 100 keys for
lookup, or more than roughly 250 keys for insert, when the program is interpreted.
Compiled, both of those numbers drop by about four: the tree is ahead by sixteen keys
for lookup and by sixty-four for insert. Below those sizes the list's lower constant
factor wins. Above them the tree's logarithmic growth wins, and the gap widens.

Outcome two from #616 (the tree wins asymptotically but loses for small N due to
constant factor) partially describes the result, but the crossover is at 64 and 256
keys, not at "thousands." Real programs that count by key, classify events, or build
indexes typically have more than 64 distinct keys once past a toy example.

`std/table` remains in the library. A table of four keys (like the log-level counter
in `examples/logs.deed`) pays less with the list, and a file that imports `std/table`
continues to work. The question of rewriting `std/table` on top of `std/map` is not
reopened by this measurement, because the two modules serve different size ranges and
making `std/table` a thin wrapper would slow down the small-N case it is designed for.
The compiled numbers narrow that range without emptying it: at sixteen keys the list
is still ahead on insert, and a handful of keys is what the module is for.

## What would change this answer

The backend was the thing, and it has now been measured rather than reasoned about.
The prediction written here was that a compiled backend would lower the tree's
constant factor and shift the crossover toward smaller N without reversing the
direction. Both halves held: the crossover moved by about a factor of four in key
count, and neither growth rate changed. The numbers are above.

What is left that could still move it: value reclamation, which today stops both
modules at a thousand keys and stops the list first. A backend that reclaims what a
program is done with would let the measurement run past the sizes where the tree is
already far ahead, so it would widen the gap rather than close it. And integer keys:
the benchmark compares with `cmp_string` because the logs example has string keys,
and a cheaper comparator takes the same fraction off both sides but a larger absolute
amount off the list, which walks more of them.

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

- Value reclamation. A compiled program gets one megabyte and gives none of it back,
  so neither module reaches a thousand keys. That is a larger question than this one
  and it is now the binding constraint above a few hundred keys.

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
