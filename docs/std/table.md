# `std/table`

_Generated from `/std/table.deed` and the module's own tests._

## Module

A keyed table, written in Deed.

`examples/logs.deed` counts how many times it has seen each level and each
source, and doing that with a plain list took two walks and a branch: one to
find out whether the key was already there, another to bump it, and a
separate case to add it the first time. That is the shape a table is for,
and the question is where a table belongs.

Not in the prelude. The test for that is whether it can be written here, the
same test `trim` passed and `contains` failed, and this file is the answer:
it can. A `record` takes type parameters, a `fn` takes type parameters, and
a `for` walks the list underneath. The compiler knows nothing about any of
this.

What it costs is what a list costs. Every lookup walks, so a table of a
thousand keys does a thousand comparisons, and the log analyser has a
handful of levels and a handful of sources. When something here holds
enough keys for that to matter, the answer is a better table rather than a
different language, and that is the point of this file being in Deed.

Measured rather than argued (#614, `crates/deed-driver/examples/interpreting.rs`):
a lookup or an insert of a key not already there costs about 350ns per key
already in the table, flat across 16, 64, 256 and 1024 keys, which is the
straight line the shape above predicts rather than something worse. `set`
costs about the same as `or_else` because it does the same walk `holds`
does before copying. Below a few hundred keys this is noise next to
anything reading a line of input; past a few thousand it is the slowest
part of a program that uses one, which is the number a tree would need to
beat rather than an assumption about when a tree is worth having.

One invariant holds it together: a key appears at most once. `set` is the
only way anything gets in and it replaces rather than appends, so `get` can
stop caring which match it finds because there is only ever one.

It lived under `examples/` to begin with, and a module's name says where it
lives, so the only way to import it was `use examples/table` and a program
outside this repository had to copy the file. Nothing about the table
changed to fix that. It ships now, so the name a program writes is the name
it has here.

## `holds`

### Behavior and limits

Whether anything under this key is already here.

Stops at the first match, which is what a `while` on the accumulator is for:
the answer cannot change once it is `true`.

### Signature

```deed
fn holds<K, V>(entries: Table<K, V>, key: K) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/table.deed`

#### `a key that was never set is missing rather than a crash`

```deed
assert holds(t, "b") == false
assert holds(t, "a")
```

#### `an empty table has nothing and says so`

```deed
assert holds(empty, "a") == false
```

#### `remove drops a key and leaves the others alone`

```deed
assert holds(without, "b") == false
```

## `get`

### Behavior and limits

What is under this key, or an error saying there is nothing.

A `Result` rather than a value, for the reason `at` hands one back: a key
that is not there is not a mistake in the caller, and nothing in this
language traps.

### Signature

```deed
fn get<K, V>(entries: Table<K, V>, key: K) -> Result<V, String>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/table.deed`

#### `a table remembers what was put in it`

```deed
assert get(t, "a") == ok(1)
assert get(t, "b") == ok(2)
```

#### `setting a key that is there replaces it rather than adding a second`

```deed
assert get(t, "a") == ok(9)
```

#### `a key that was never set is missing rather than a crash`

```deed
assert get(t, "b") == err("no such key")
```

#### `a fallback turns the two cases of counting into one`

```deed
assert get(twice, "e") == ok(2)
```

#### `the key type is whatever was used, not just text`

```deed
assert get(t, 20) == ok("twenty")
```

## `set`

### Behavior and limits

This key holding this value, whether or not it held one before.

Replaces rather than appends, which is the invariant every other function
here leans on.

### Signature

```deed
fn set<K, V>(entries: Table<K, V>, key: K, value: V) -> Table<K, V>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/table.deed`

#### `a table remembers what was put in it`

```deed
let t = set(set([], "a", 1), "b", 2)
```

#### `setting a key that is there replaces it rather than adding a second`

```deed
let t = set(set([], "a", 1), "a", 9)
```

#### `a key that was never set is missing rather than a crash`

```deed
let t = set([], "a", 1)
```

#### `a fallback turns the two cases of counting into one`

```deed
let once = set(empty, "e", or_else(empty, "e", 0) + 1)
let twice = set(once, "e", or_else(once, "e", 0) + 1)
```

#### `the keys and the values come back in the order they went in`

```deed
let t = set(set(set([], "a", 1), "b", 2), "c", 3)
```

#### `the key type is whatever was used, not just text`

```deed
let t = set(set([], 10, "ten"), 20, "twenty")
```

#### `remove drops a key and leaves the others alone`

```deed
let t = set(set(set([], "a", 1), "b", 2), "c", 3)
```

## `or_else`

### Behavior and limits

What is under this key, or `fallback` if there is nothing.

The shape a counter wants: `set(t, k, or_else(t, k, 0) + 1)` adds the first
one and bumps the rest without asking which case it is in.

### Signature

```deed
fn or_else<K, V>(entries: Table<K, V>, key: K, fallback: V) -> V
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/table.deed`

#### `a fallback turns the two cases of counting into one`

```deed
let once = set(empty, "e", or_else(empty, "e", 0) + 1)
let twice = set(once, "e", or_else(once, "e", 0) + 1)
```

## `keys`

### Behavior and limits

Every key in insertion order.

Replacing a value keeps the key where it was, because `set` rewrites in
place rather than removing and appending.

### Signature

```deed
fn keys<K, V>(entries: Table<K, V>) -> List<K>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/table.deed`

#### `the keys and the values come back in the order they went in`

```deed
assert keys(t) == ["a", "b", "c"]
```

#### `the key type is whatever was used, not just text`

```deed
assert keys(t) == [10, 20]
```

#### `an empty table has nothing and says so`

```deed
assert keys(empty) == []
```

#### `remove drops a key and leaves the others alone`

```deed
assert keys(without) == ["a", "c"]
```

## `values`

### Behavior and limits

Every value in the same order `keys` answers its keys.

### Signature

```deed
fn values<K, V>(entries: Table<K, V>) -> List<V>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/table.deed`

#### `the keys and the values come back in the order they went in`

```deed
assert values(t) == [1, 2, 3]
```

#### `an empty table has nothing and says so`

```deed
assert values(empty) == []
```

#### `remove drops a key and leaves the others alone`

```deed
assert values(without) == [1, 3]
```

## `size`

### Behavior and limits

How many entries are in the table.

The count is the underlying list's length, which is also the reason lookup
cost stays linear in the number of keys.

### Signature

```deed
fn size<K, V>(entries: Table<K, V>) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/table.deed`

#### `a table remembers what was put in it`

```deed
assert size(t) == 2
```

#### `setting a key that is there replaces it rather than adding a second`

```deed
assert size(t) == 1
```

#### `a fallback turns the two cases of counting into one`

```deed
assert size(twice) == 1
```

#### `an empty table has nothing and says so`

```deed
assert size(empty) == 0
```

#### `remove drops a key and leaves the others alone`

```deed
assert size(without) == 2
```

## `remove`

### Behavior and limits

Without this key, whether or not it was there. Missing is quiet: the table
after is the table that does not hold the key, and asking twice is the same
as asking once. `set` is the only way anything gets in; this is the only way
anything gets out.

### Signature

```deed
fn remove<K, V>(entries: Table<K, V>, key: K) -> Table<K, V>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/table.deed`

#### `remove drops a key and leaves the others alone`

```deed
let without = remove(t, "b")
assert remove(without, "b") == without
assert remove([], "a") == []
```
