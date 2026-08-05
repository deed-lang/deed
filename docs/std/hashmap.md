# `std/hashmap`

_Generated from `/std/hashmap.deed` and the module's own tests._

## Module

A hash map, written in Deed.

`std/table` is a list its `get` walks, so it costs about 350ns per key
already in it. That is fine for a handful and it is the slowest part of any
program with a few thousand. This file is the answer that file asked for:
"when something here holds enough keys for that to matter, the answer is a
better table rather than a different language".

What made it writable was `hash`, which arrived because taking a value of
any shape apart is one of the few things this language cannot say about
itself. Everything else was already here: a `List` is contiguous with O(1)
indexed access, `at` reads one element, and a `record` and a `fn` both take
type parameters.

The shape is buckets, and a bucket is a `std/table`. A lookup hashes the
key, indexes straight to its bucket, and walks only that one. With the keys
spread evenly, a bucket holds `n / BUCKETS` entries, so a lookup is a
constant walk rather than a walk over everything.

What it costs is `set`. Nothing here mutates, so putting a key in rebuilds
the list of buckets: `BUCKETS` steps whichever key it was, plus the bucket
itself. That is a constant rather than the length of the map, which is the
trade this file is: `get` stops growing and `set` stops being free.

Measured, building a map of n keys and then reading all n back, against
`std/table` doing the same, through the interpreter. Least of three runs,
with the cost of building the list of keys measured separately and taken
off both sides:

    keys      table    hashmap
    250        18ms       46ms
    500        62ms       96ms
    1000      232ms      166ms
    2000      939ms      335ms

The table roughly quadruples as the keys double, which is the quadratic its
own header predicts. This roughly doubles. **They cross at around seven
hundred keys, and below that the table is faster**, because a constant of
sixty-four buckets is worse than walking a list that is shorter than
sixty-four. So this is not a better table; it is the one that keeps working
when a table stops, and `std/table` is still the right answer for a handful
of keys.

An earlier version of this comment claimed the map was flat and nineteen
times faster at every size. That measurement was taken before this module
was in `crates/deed-driver/src/shipped.rs`, so the program under it did not
check and `deed test` was timed refusing to run. The numbers above are what
it does when it runs.

One thing this deliberately does not do is grow. The bucket count is fixed,
so a map of a hundred thousand keys has thousand-deep buckets and is a slow
list again. Growing means rehashing every key on the turn that crosses the
threshold, and the honest reason not to write that yet is that nothing here
has measured needing it.

Compiled, this stops between three and four hundred keys. Nothing is given
back there, so what a program allocates in total is what its memory reached,
and `set` allocates the whole bucket list every time. The interpreter has no
such ceiling and the table above was taken through it.

It used to stop at fifty, and finding out why is what
`crates/deed-driver/tests/map_memory.rs` is: an insert cost nineteen
kilobytes and seventeen of them were `range`, which read its own
accumulator with `length(out)` and so could not be built in one list. The
bucket list itself was five hundred bytes. Nothing about the language
changed; one walk stopped mentioning its accumulator twice. The rule has
since learned to read a length, so `range` is written the obvious way again
and stopped rebuilding a record a turn as well, which is the second time the
ceiling moved.

That is the same limit
`design/decisions/2026-07-31-compiled-memory-reclamation.md` measured from
the other side, met by the first structure written to need it, which is what
`design/hash-map-requirements.md` said would decide what the reuse work does
next.

## `buckets`

### Behavior and limits

How many buckets a map has, for as long as it has one number.

A power of two would let the index be a mask rather than a remainder, and
there is no bitwise operator in this language, so it is a remainder either
way and the number is chosen to be a reasonable spread rather than to suit
an operation that does not exist.

### Signature

```deed
fn buckets() -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `the shape underneath is buckets, and a bucket is a table`

```deed
assert length(m) == buckets()
```

## `empty`

### Behavior and limits

A map with nothing in it.

It takes the key and the value it will never hold, and that is not an
oversight. `DEED4023` refuses a function whose type parameters appear only
in its return type, because a return type is what a call produces rather
than something it can be worked out from. `std/table` needs no such thing
because an empty table is `[]` and a list literal takes its element type
from where it is used; a map is a list of lists, and the inner one has
nothing to take a type from.

`repeat` of nothing is what makes the shape without holding anything: the
element decides the type and the count decides that there is none of it.

### Signature

```deed
fn empty<K, V>(key: K, value: V) -> List<List<Entry<K, V>>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `many keys, some of them sharing a bucket`

```deed
let many = for n in range(50) with m = empty(0, 0) {
```

#### `an empty map holds nothing and answers everything`

```deed
let m = empty("a", 1)
```

## `holding`

### Behavior and limits

A map holding one key, which is how most of them start.

### Signature

```deed
fn holding<K, V>(key: K, value: V) -> List<List<Entry<K, V>>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `a key put in comes back out`

```deed
let m = set(holding("a", 1), "b", 2)
```

#### `a key not there says so rather than trapping`

```deed
let m = holding("a", 1)
```

#### `putting a key twice replaces it and does not grow`

```deed
let m = set(holding("a", 1), "a", 2)
```

#### `keys that land in one bucket still tell each other apart`

```deed
let m = set(holding(1, 10), 65, 650)
```

#### `a record is a key like anything else`

```deed
let m = holding(Entry { key: "a", value: 1 }, "held")
```

#### `the shape underneath is buckets, and a bucket is a table`

```deed
let m = holding("a", 1)
```

#### `values come back beside their keys`

```deed
let m = set(holding("a", 1), "b", 2)
```

#### `range counts up from zero`

```deed
assert length(entries(holding("a", 1))) == 1
```

## `size`

### Behavior and limits

How many keys are in it.

Walks the buckets, which is `buckets()` steps plus the keys. There is
nowhere to keep a count: an alias is the list it names, so there is no field
beside it to put one in.

### Signature

```deed
fn size<K, V>(map: List<List<Entry<K, V>>>) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `a key put in comes back out`

```deed
assert size(m) == 2
```

#### `putting a key twice replaces it and does not grow`

```deed
assert size(m) == 1
```

#### `keys that land in one bucket still tell each other apart`

```deed
assert size(m) == 2
```

#### `many keys, some of them sharing a bucket`

```deed
assert size(many) == 50
```

#### `an empty map holds nothing and answers everything`

```deed
assert size(m) == 0
```

## `bucket_of`

### Behavior and limits

Which bucket a key belongs in.

`hash` can answer with any `Int`, including a negative one, and a negative
remainder is not an index. Adding the count back is what turns it into one,
and it is written this way rather than by negating because negating the
smallest `Int` is the one case that has no answer.

### Signature

```deed
fn bucket_of<K>(key: K) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `keys that land in one bucket still tell each other apart`

```deed
assert bucket_of(1) == bucket_of(65)
```

#### `a bucket is never a negative index`

```deed
assert bucket_of("a") >= 0
assert bucket_of("zzzzzzzz") >= 0
assert bucket_of(0 - 7) >= 0
assert bucket_of("a") < 64
```

#### `the shape underneath is buckets, and a bucket is a table`

```deed
assert length(bucket_at(m, bucket_of("a"))) == 1
```

## `bucket_at`

### Behavior and limits

The entries sharing a key's bucket.

### Signature

```deed
fn bucket_at<K, V>(map: List<List<Entry<K, V>>>, index: Int) -> List<Entry<K, V>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `the shape underneath is buckets, and a bucket is a table`

```deed
assert length(bucket_at(m, bucket_of("a"))) == 1
assert length(bucket_at(m, 0 - 1)) == 0
```

## `get`

### Behavior and limits

What is under this key, or an error saying there is nothing.

The `Result` `std/table` hands back, for the same reason: a key that is not
there is not a mistake in the caller, and nothing in this language traps.

### Signature

```deed
fn get<K, V>(map: List<List<Entry<K, V>>>, key: K) -> Result<V, String>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `an empty map holds nothing and answers everything`

```deed
assert get(m, "a") == err("no such key")
```

## `holds`

### Behavior and limits

Whether anything under this key is already here.

### Signature

```deed
fn holds<K, V>(map: List<List<Entry<K, V>>>, key: K) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `a key not there says so rather than trapping`

```deed
assert !holds(m, "b")
```

#### `an empty map holds nothing and answers everything`

```deed
assert !holds(m, "a")
```

## `or_else`

### Behavior and limits

What is under this key, or `fallback` if there is nothing.

The shape a counter wants, the same one `std/table` provides:
`set(m, k, or_else(m, k, 0) + 1)` adds the first one and bumps the rest
without asking which case it is in.

### Signature

```deed
fn or_else<K, V>(map: List<List<Entry<K, V>>>, key: K, fallback: V) -> V
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `a key put in comes back out`

```deed
assert or_else(m, "a", 0) == 1
assert or_else(m, "b", 0) == 2
```

#### `a key not there says so rather than trapping`

```deed
assert or_else(m, "b", 9) == 9
```

#### `putting a key twice replaces it and does not grow`

```deed
assert or_else(m, "a", 0) == 2
```

#### `keys that land in one bucket still tell each other apart`

```deed
assert or_else(m, 1, 0) == 10
assert or_else(m, 65, 0) == 650
```

#### `many keys, some of them sharing a bucket`

```deed
assert or_else(many, 0, 0 - 1) == 0
assert or_else(many, 49, 0 - 1) == 98
```

#### `a record is a key like anything else`

```deed
assert or_else(m, Entry { key: "a", value: 1 }, "") == "held"
assert or_else(m, Entry { key: "a", value: 2 }, "") == ""
```

## `set`

### Behavior and limits

This key holding this value, whether or not it held one before.

Replaces rather than appends, which is the invariant everything else here
leans on: a key appears at most once, so `get` can stop at the first match.

### Signature

```deed
fn set<K, V>(map: List<List<Entry<K, V>>>, key: K, value: V) -> List<List<Entry<K, V>>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `a key put in comes back out`

```deed
let m = set(holding("a", 1), "b", 2)
```

#### `putting a key twice replaces it and does not grow`

```deed
let m = set(holding("a", 1), "a", 2)
```

#### `keys that land in one bucket still tell each other apart`

```deed
let m = set(holding(1, 10), 65, 650)
```

#### `many keys, some of them sharing a bucket`

```deed
set(m, n, n + n)
```

#### `values come back beside their keys`

```deed
let m = set(holding("a", 1), "b", 2)
```

## `written`

### Behavior and limits

One bucket with the key in it, replacing what was there or adding it.

### Signature

```deed
fn written<K, V>(bucket: List<Entry<K, V>>, key: K, value: V, had: Bool)
    -> List<Entry<K, V>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `a bucket that already holds the key is written rather than appended`

```deed
assert length(written(bucket, "a", 2, true)) == 1
assert length(written(bucket, "b", 2, false)) == 2
```

## `entries`

### Behavior and limits

Every entry, in no particular order.

The one walk here that carries the accumulator into an inner walk, which is
the shape `design/decisions/2026-08-04-a-walk-that-only-pushes.md` cannot
build in one list. Written once so that `keys` and `values` are the shape it
can.

### Signature

```deed
fn entries<K, V>(map: List<List<Entry<K, V>>>) -> List<Entry<K, V>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `range counts up from zero`

```deed
assert length(entries(holding("a", 1))) == 1
```

## `keys`

### Behavior and limits

Every key, in no particular order.

Unlike `std/table`, which keeps insertion order because it is a list. A hash
map's order is the hash's, so nothing here promises one and a caller that
needs an order sorts.

### Signature

```deed
fn keys<K, V>(map: List<List<Entry<K, V>>>) -> List<K>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `putting a key twice replaces it and does not grow`

```deed
assert length(keys(m)) == 1
```

#### `many keys, some of them sharing a bucket`

```deed
assert length(keys(many)) == 50
```

#### `an empty map holds nothing and answers everything`

```deed
assert length(keys(m)) == 0
```

#### `values come back beside their keys`

```deed
assert length(keys(m)) == 2
```

## `values`

### Behavior and limits

Every value, in the order `keys` gives their keys.

### Signature

```deed
fn values<K, V>(map: List<List<Entry<K, V>>>) -> List<V>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `an empty map holds nothing and answers everything`

```deed
assert length(values(m)) == 0
```

#### `values come back beside their keys`

```deed
let total = for v in values(m) with sum = 0 {
```

## `range`

### Behavior and limits

The numbers from zero up to `count`, which a walk needs before it can walk.

`for` walks a list that already exists, and the buckets are indexed rather
than held, so something has to turn a count into a list first. `repeat`
gives the length and the length of what has been built so far gives the
position.

This used to carry a record of a position and a list, because the rule in
`design/decisions/2026-08-04-a-walk-that-only-pushes.md` counted every
mention of the accumulator and `push(out, length(out))` mentions it twice.
A range of sixty four cost sixteen kilobytes rather than half of one until
the position moved into the record.
`design/decisions/2026-08-05-a-walk-may-read-its-own-length.md` let the
walk read its own length instead, so the record went away and the numbers
did not move.

### Signature

```deed
fn range(count: Int) -> List<Int>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/hashmap.deed`

#### `many keys, some of them sharing a bucket`

```deed
let many = for n in range(50) with m = empty(0, 0) {
```

#### `range counts up from zero`

```deed
assert length(range(3)) == 3
assert at(range(3), 0) == ok(0)
assert at(range(3), 2) == ok(2)
```
