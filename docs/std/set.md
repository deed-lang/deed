# `std/set`

_Generated from `/std/set.deed` and the module's own tests._

## Module

A set, written on top of `std/hashmap`.

A set is a map whose values carry nothing, and this is that map with the
values hidden. Everything a set does that a map does not, union and
intersection and difference, is a walk over one side asking the other
whether it holds something, which is the operation the map is already fast
at.

The shape is written out rather than aliased for the same reason
`std/hashmap` writes its own out: an alias over a list of lists does not
expand where it is used, so a signature naming it would say less than this
one does. What that costs is a type that is a mouthful; what it buys is that
a set is a map and a program can say so.

A set has no `Empty`. Every constructor takes a sample of the element type,
because an empty list takes its element type from where it is used and there
is nowhere here for it to take one from. That is the same shape
`std/hashmap`'s `empty` has and for the same reason.

## `none`

### Behavior and limits

A set holding nothing, shaped by a sample of what it would hold.

### Signature

```deed
fn none<T>(sample: T) -> List<List<Entry<T, Bool>>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `an empty set holds nothing and says so`

```deed
let s = none(0)
```

#### `a set is within one that holds everything it holds`

```deed
assert within(none("z"), big)
```

## `one`

### Behavior and limits

A set holding one item, which is how most of them start.

### Signature

```deed
fn one<T>(item: T) -> List<List<Entry<T, Bool>>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `a set holds an item once however many times it is put in`

```deed
let s = including(including(one("a"), "b"), "a")
```

#### `taking an item out leaves the rest`

```deed
let s = including(including(one(1), 2), 3)
```

#### `union, intersection and difference agree with each other`

```deed
let left = including(including(one(1), 2), 3)
let right = including(including(one(3), 4), 5)
```

#### `a set is within one that holds everything it holds`

```deed
let small = including(one("a"), "b")
let big = including(including(one("a"), "b"), "c")
```

#### `a set is the map it is made of`

```deed
let s = including(one("a"), "b")
```

## `including`

### Behavior and limits

The same set with `item` in it, whether or not it was there before.

Not `with`, which is how a handler is installed and therefore a word the
grammar has already spoken for.

### Signature

```deed
fn including<T>(subject: List<List<Entry<T, Bool>>>, item: T)
    -> List<List<Entry<T, Bool>>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `a set holds an item once however many times it is put in`

```deed
let s = including(including(one("a"), "b"), "a")
```

#### `taking an item out leaves the rest`

```deed
let s = including(including(one(1), 2), 3)
```

#### `union, intersection and difference agree with each other`

```deed
let left = including(including(one(1), 2), 3)
let right = including(including(one(3), 4), 5)
```

#### `a set is within one that holds everything it holds`

```deed
let small = including(one("a"), "b")
let big = including(including(one("a"), "b"), "c")
```

#### `a set is the map it is made of`

```deed
let s = including(one("a"), "b")
```

## `has`

### Behavior and limits

Whether the set holds `item`.

### Signature

```deed
fn has<T>(subject: List<List<Entry<T, Bool>>>, item: T) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `a set holds an item once however many times it is put in`

```deed
assert has(s, "a")
assert has(s, "b")
assert !has(s, "c")
```

#### `an empty set holds nothing and says so`

```deed
assert !has(s, 0)
```

#### `taking an item out leaves the rest`

```deed
assert has(smaller, 1)
assert !has(smaller, 2)
assert has(smaller, 3)
```

#### `union, intersection and difference agree with each other`

```deed
assert has(intersection(left, right, 0), 3)
assert has(difference(left, right, 0), 1)
assert !has(difference(left, right, 0), 3)
```

## `count`

### Behavior and limits

How many items the set holds.

### Signature

```deed
fn count<T>(subject: List<List<Entry<T, Bool>>>) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `a set holds an item once however many times it is put in`

```deed
assert count(s) == 2
```

#### `an empty set holds nothing and says so`

```deed
assert count(s) == 0
```

#### `taking an item out leaves the rest`

```deed
assert count(smaller) == 2
assert count(without(smaller, 9)) == 2
```

#### `union, intersection and difference agree with each other`

```deed
assert count(union(left, right)) == 5
assert count(intersection(left, right, 0)) == 1
assert count(difference(left, right, 0)) == 2
```

## `items`

### Behavior and limits

Every item, in the order the map gives them.

Not sorted: a set has no order of its own, and inventing one would need a
comparison this cannot ask for.

### Signature

```deed
fn items<T>(subject: List<List<Entry<T, Bool>>>) -> List<T>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `an empty set holds nothing and says so`

```deed
assert length(items(s)) == 0
```

## `without`

### Behavior and limits

The same set without `item`, which is a rebuild rather than a removal.

`std/hashmap` has no way to take a key out, and the honest reason is that
nothing has needed one: a map is rebuilt from its entries here, which is
linear in the size of the set rather than in the size of one bucket. A
`remove` on the map would make this a bucket-sized walk, and that is the day
to write it.

### Signature

```deed
fn without<T>(subject: List<List<Entry<T, Bool>>>, item: T) -> List<List<Entry<T, Bool>>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `taking an item out leaves the rest`

```deed
let smaller = without(s, 2)
assert count(without(smaller, 9)) == 2
```

## `union`

### Behavior and limits

Everything in either set.

### Signature

```deed
fn union<T>(left: List<List<Entry<T, Bool>>>, right: List<List<Entry<T, Bool>>>)
    -> List<List<Entry<T, Bool>>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `union, intersection and difference agree with each other`

```deed
assert count(union(left, right)) == 5
```

## `intersection`

### Behavior and limits

Everything in both sets.

### Signature

```deed
fn intersection<T>(
    left: List<List<Entry<T, Bool>>>,
    right: List<List<Entry<T, Bool>>>,
    sample: T,
)
    -> List<List<Entry<T, Bool>>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `union, intersection and difference agree with each other`

```deed
assert count(intersection(left, right, 0)) == 1
assert has(intersection(left, right, 0), 3)
```

## `difference`

### Behavior and limits

Everything in the first set that is not in the second.

The sample is here for the same reason it is on `none`: the answer can be
empty, and an empty set has to be shaped by something.

### Signature

```deed
fn difference<T>(
    left: List<List<Entry<T, Bool>>>,
    right: List<List<Entry<T, Bool>>>,
    sample: T,
)
    -> List<List<Entry<T, Bool>>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `union, intersection and difference agree with each other`

```deed
assert count(difference(left, right, 0)) == 2
assert has(difference(left, right, 0), 1)
assert !has(difference(left, right, 0), 3)
```

## `within`

### Behavior and limits

Whether every item of the first set is in the second.

### Signature

```deed
fn within<T>(left: List<List<Entry<T, Bool>>>, right: List<List<Entry<T, Bool>>>) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `a set is within one that holds everything it holds`

```deed
assert within(small, big)
assert !within(big, small)
assert within(small, small)
assert within(none("z"), big)
```

## `entries_of`

### Behavior and limits

How many entries the underlying map holds, which is what `count` reads.

Exposed because a set built by hand out of `std/hashmap` and one built here
are the same value, and a program mixing the two should be able to say so.

### Signature

```deed
fn entries_of<T>(subject: List<List<Entry<T, Bool>>>) -> List<Entry<T, Bool>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/set.deed`

#### `a set is the map it is made of`

```deed
assert length(entries_of(s)) == 2
```
