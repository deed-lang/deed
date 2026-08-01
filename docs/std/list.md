# `std/list`

_Generated from `/std/list.deed` and the module's own tests._

## Module

A list library, written in Deed rather than built into the compiler.

This file is the point of the last three changes. Generic functions made
`map` writable once instead of once per element type. Generic types made
`Option` declarable instead of built in. Row variables make the callback
able to do something.

Before row variables there were two ways to write `map`, and both were
wrong. One took `Fn(A) -> B`, which promises to perform nothing, so the
callback could not log or read a file. The other took
`Fn(A) uses Log.note -> B`, which works for exactly one effect and needs a
second copy for the next one.

`uses r` is a row variable: it stands for whatever the callback performs,
and `uses r` on the function passes that through to its own row. One `map`,
any callback, and a caller that has to declare what its own callback does.

Nothing in here is special to the compiler. Every function below could have
been written by anybody, which is the first time that has been true of
anything in this repository.

It lived under `examples/` for a while, and a module's name says where it
lives, so the only way to import it was `use examples/list` and a program
outside this repository had to copy the file. Nothing about the library
changed to fix that. It ships now, so the name a program writes is the name
it has here.

## `map`

### Behavior and limits

Turn each element into something else. The classic one, and the one that
could not be written until now.

### Signature

```deed
fn map<A, B, uses r>(items: List<A>, step: Fn(A) uses r -> B) -> List<B>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `map turns each element into something else, in order`

```deed
assert map([1, 2, 3], |n: Int| n + n) == [2, 4, 6]
assert map(["a", "b"], |word: String| word + "!") == ["a!", "b!"]
assert map([7], |n: Int| n) == [7]
```

## `map_at`

### Behavior and limits

The same, and the callback is told where the element was.

This is the test for whether a `for` should be the thing that knows a
position rather than the library. It could not be written before, because
everything in this file is a `for` and a `for` could not say where it was,
so a callback could not be told something the walk did not know. Now it can,
and the library builds the indexed form rather than the language growing a
second one.

### Signature

```deed
fn map_at<A, B, uses r>(items: List<A>, step: Fn(Int, A) uses r -> B) -> List<B>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `map_at counts from zero and stops one short of the length`

```deed
assert map_at([10, 20, 30], |index: Int, n: Int| index) == [0, 1, 2]
assert map_at(["a", "b"], |index: Int, word: String| word + to_string(index)) == ["a0", "b1"]
```

## `filter`

### Behavior and limits

Keep the ones that pass. A `for` is a fold, so this is the accumulator
growing or not growing, which is what a filter is.

### Signature

```deed
fn filter<T, uses r>(items: List<T>, keep: Fn(T) uses r -> Bool) -> List<T>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `filter keeps the order it was given, and can keep none of it`

```deed
assert filter([3, 1, 2], |n: Int| n > 1) == [3, 2]
assert filter([1, 2, 3], |n: Int| n > 9) == []
```

## `filter_at`

### Behavior and limits

The same, and the callback is told where the element was.

`map_at` is the first of these. Filtering by position is the second shape
somebody reaches for once they have one, and it is the same four lines with
a keep instead of a step. Written here rather than waiting for a third walk
to grow its own indexed form.

### Signature

```deed
fn filter_at<T, uses r>(items: List<T>, keep: Fn(Int, T) uses r -> Bool) -> List<T>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `filter_at keeps by position and by value`

```deed
assert filter_at([10, 20, 30, 40], |index: Int, n: Int| index < 2) == [10, 20]
assert filter_at([10, 20, 30, 40], |index: Int, n: Int| n > 25) == [30, 40]
assert filter_at([10, 20, 30], |index: Int, n: Int| index == n / 10 - 1) == [10, 20, 30]
assert filter_at([], |index: Int, n: Int| true) == []
```

## `fold`

### Behavior and limits

The general one, and the one `for` already is. Written out anyway, because
a library that has `map` and `filter` and not `fold` is a library that
stops working the moment somebody wants a sum.

### Signature

```deed
fn fold<T, S, uses r>(items: List<T>, start: S, step: Fn(S, T) uses r -> S) -> S
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `fold answers with what it started from when the walk does nothing`

```deed
assert fold([1, 2, 3], 0, |total: Int, n: Int| total + n) == 6
assert fold(["a", "b"], "", |joined: String, word: String| joined + word) == "ab"
assert fold([], 7, |total: Int, n: Int| total + n) == 7
```

## `fold_at`

### Behavior and limits

The same, and the callback is told where the element was.

`map_at` and `filter_at` cover turning and keeping. Folding with a position
is the third shape: a running total that depends on index, without inventing
a second accumulator just to count.

### Signature

```deed
fn fold_at<T, S, uses r>(items: List<T>, start: S, step: Fn(S, Int, T) uses r -> S) -> S
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `fold_at hands the index into the step`

```deed
assert fold_at([10, 20, 30], 0, |total: Int, index: Int, n: Int| total + index) == 3
assert fold_at([10, 20, 30], 0, |total: Int, index: Int, n: Int| total + n) == 60
assert fold_at([], 7, |total: Int, index: Int, n: Int| total + index + n) == 7
```

## `any`

### Behavior and limits

The two that wanted to stop. `while` is read before each turn with the
accumulator in scope, so these say what they mean in the head of the loop
instead of carrying a branch through the body whose only job is to notice
that the answer is already in.

This is why the language grew it rather than the library working around it.
The workaround was writable and it was here: both of these used to open with
`if found` or `if so_far`, which is control flow inside a fold, which is the
thing a fold exists to not have. The walk also could not stop, so `any` over
a thousand elements took a thousand turns to find the first one.

### Signature

```deed
fn any<T, uses r>(items: List<T>, matches: Fn(T) uses r -> Bool) -> Bool
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `any and all disagree about an empty list on purpose`

```deed
assert any([], |n: Int| true) == false
```

## `all`

### Behavior and limits

Whether every element matches.

Stops at the first failure for the same reason `any` stops at the first hit.
An empty list answers `true`, which is the usual "all of nothing" case.

### Signature

```deed
fn all<T, uses r>(items: List<T>, matches: Fn(T) uses r -> Bool) -> Bool
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `any and all disagree about an empty list on purpose`

```deed
assert all([], |n: Int| false) == true
```

## `count_where`

### Behavior and limits

How many elements match.

This is `filter` without building the kept list, so the callback still runs
on every element and only the count is accumulated.

### Signature

```deed
fn count_where<T, uses r>(items: List<T>, matches: Fn(T) uses r -> Bool) -> Int
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `filter keeps the order it was given, and can keep none of it`

```deed
assert count_where([3, 1, 2], |n: Int| n > 1) == 2
assert count_where([1, 2, 3], |n: Int| n > 9) == 0
```

## `filtered_with`

### Behavior and limits

Filter, and say something about the ones that did not make it. Two
callbacks, and `r` names both of their rows.

One variable in two parameters does not force the two callbacks to agree.
It stands for whatever was passed at each place it appears, and the caller
is charged with the sum. So `keep` can be pure while `dropped` logs, and the
caller declares what `dropped` does and nothing else.

### Signature

```deed
fn filtered_with<T, uses r>(
    items: List<T>,
    keep: Fn(T) uses r -> Bool,
    dropped: Fn(T) uses r -> (),
)
    -> List<T>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `filtered_with keeps what filter keeps`

```deed
assert filtered_with([1, 0 - 2, 3], |n: Int| n > 0, |n: Int| ()) == [1, 3]
```

## `first`

### Behavior and limits

These two need no callback and so no row variable. Worth having next to the
rest so that the row on the others reads as a decision rather than as
something every signature in the file happens to carry.

### Signature

```deed
fn first<T>(items: List<T>) -> Result<T, String>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `the ends of a list, and a list with no ends`

```deed
assert first([1, 2, 3]) == ok(1)
assert first(["only"]) == ok("only")
assert first([]) == err("index 0 is outside a list of 0")
```

## `last`

### Behavior and limits

The last element, or the same out-of-range error `at` gives on an empty list.

### Signature

```deed
fn last<T>(items: List<T>) -> Result<T, String>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `the ends of a list, and a list with no ends`

```deed
assert last([1, 2, 3]) == ok(3)
assert last(["only"]) == ok("only")
assert last([]) == err("index -1 is outside a list of 0")
```

## `reversed`

### Behavior and limits

The same items, in the opposite order.

### Signature

```deed
fn reversed<T>(items: List<T>) -> List<T>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `reversed and prepend put things at the front`

```deed
assert reversed([1, 2, 3]) == [3, 2, 1]
assert reversed([1]) == [1]
assert reversed([]) == []
```

## `prepend`

### Behavior and limits

`front` followed by the list that was already there.

### Signature

```deed
fn prepend<T>(items: List<T>, front: T) -> List<T>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `reversed and prepend put things at the front`

```deed
assert prepend([2, 3], 1) == [1, 2, 3]
assert prepend([], 1) == [1]
```

## `find`

### Behavior and limits

The first that matches, or nothing. `any` only says whether; this hands the
element back. Stops on the first hit, which is the same reason `any` grew a
`while` rather than walking the rest of a finished answer.

### Signature

```deed
fn find<T, uses r>(items: List<T>, matches: Fn(T) uses r -> Bool) -> Result<T, String>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `find hands back the first match, or says nothing matched`

```deed
assert find([1, 2, 3], |n: Int| n > 1) == ok(2)
assert find([1, 2, 3], |n: Int| n > 9) == err("nothing matched")
assert find([], |n: Int| true) == err("nothing matched")
assert find(["a", "bb", "c"], |word: String| length(word) > 1) == ok("bb")
```

## `take`

### Behavior and limits

The first `upto` elements, in the order they were given. A walk that already
has enough stops asking for more, so a long list and a small prefix cost the
prefix rather than the list.

Negative and oversize are both quiet: less than nothing is nothing, and more
than there is is everything. The same answers `repeat` gives about a count
that is not a length, and for the same reason — the call is usually
`take(xs, n)` where `n` came from arithmetic, not from a proof.

### Signature

```deed
fn take<T>(items: List<T>, upto: Int) -> List<T>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `take keeps a prefix and drop keeps the rest`

```deed
assert take([1, 2, 3], 2) == [1, 2]
assert take([1, 2, 3], 0) == []
assert take([1, 2, 3], 9) == [1, 2, 3]
assert take([], 3) == []
assert take([1, 2, 3, 4], 2) == [1, 2]
```

## `drop`

### Behavior and limits

Everything after the first `count` elements. The other half of `take`: the
two together rebuild the list they split, which is the property that keeps
either one honest.

### Signature

```deed
fn drop<T>(items: List<T>, count: Int) -> List<T>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `take keeps a prefix and drop keeps the rest`

```deed
assert drop([1, 2, 3], 1) == [2, 3]
assert drop([1, 2, 3], 0) == [1, 2, 3]
assert drop([1, 2, 3], 9) == []
assert drop([], 1) == []
assert drop([1, 2, 3, 4], 2) == [3, 4]
```

## `concat`

### Behavior and limits

One list after another. The building block under `flatten`, and the thing a
caller reaches for when `push` is one element too small.

### Signature

```deed
fn concat<T>(left: List<T>, right: List<T>) -> List<T>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `concat puts one list after another`

```deed
assert concat([1, 2], [3, 4]) == [1, 2, 3, 4]
assert concat([], [1]) == [1]
assert concat([1], []) == [1]
assert concat([], []) == []
```

## `flatten`

### Behavior and limits

One level of nesting undone. Nested `for` is writable, but every program that
wants it writes the same two lines, so they live here once.

### Signature

```deed
fn flatten<T>(items: List<List<T>>) -> List<T>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `flatten undoes one level of nesting`

```deed
assert flatten([[1, 2], [3], [], [4, 5]]) == [1, 2, 3, 4, 5]
assert flatten([]) == []
assert flatten([[], []]) == []
```

## `partition`

### Behavior and limits

Both sides of a filter, kept and rejected, each in input order.

### Signature

```deed
fn partition<T, uses r>(items: List<T>, keep: Fn(T) uses r -> Bool) -> Parts<T>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `partition keeps both sides in order`

```deed
let parts = partition([1, 2, 3, 4], |n: Int| n > 2)
let none = partition([1, 2], |n: Int| n > 9)
let empty = partition([], |n: Int| true)
```

## `zip`

### Behavior and limits

Two lists walked together into pairs.

Stops when either side runs out, so anything past the shorter length is
dropped rather than padded.

### Signature

```deed
fn zip<A, B>(left: List<A>, right: List<B>) -> List<Pair<A, B>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `zip stops when either list runs out`

```deed
assert zip([1, 2, 3], ["a", "b"]) == [Pair { left: 1, right: "a" }, Pair { left: 2, right: "b" }]
assert zip([1], ["a", "b", "c"]) == [Pair { left: 1, right: "a" }]
assert zip([], ["a"]) == []
assert zip([1], []) == []
```

## `enumerate`

### Behavior and limits

Each element paired with where it was. `map_at` already hands the index into
a callback; this is the same walk that keeps the pair rather than calling
out. Useful when the next step wants a list of pairs rather than a callback.

### Signature

```deed
fn enumerate<T>(items: List<T>) -> List<Pair<Int, T>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `enumerate pairs each element with its index`

```deed
assert enumerate([10, 20, 30]) == [
assert enumerate(["a"]) == [Pair { left: 0, right: "a" }]
assert enumerate([]) == []
```

## `windows`

### Behavior and limits

Overlapping slices of `size` consecutive elements, in order. A size that is
not positive, or longer than the list, is an empty answer rather than an
error: the call is usually `windows(xs, n)` where `n` came from arithmetic.

### Signature

```deed
fn windows<T>(items: List<T>, size: Int) -> List<List<T>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `windows walks overlapping slices`

```deed
assert windows([1, 2, 3, 4], 2) == [[1, 2], [2, 3], [3, 4]]
assert windows([1, 2, 3], 3) == [[1, 2, 3]]
assert windows([1, 2], 3) == []
assert windows([1, 2, 3], 0) == []
assert windows([], 1) == []
```

## `chunks`

### Behavior and limits

Non-overlapping slices of `size` consecutive elements, in order. The last
chunk may be shorter when the length does not divide evenly. A size that is
not positive is an empty answer, matching `windows`.

### Signature

```deed
fn chunks<T>(items: List<T>, size: Int) -> List<List<T>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `chunks walks non-overlapping slices`

```deed
assert chunks([1, 2, 3, 4], 2) == [[1, 2], [3, 4]]
assert chunks([1, 2, 3, 4, 5], 2) == [[1, 2], [3, 4], [5]]
assert chunks([1, 2, 3], 3) == [[1, 2, 3]]
assert chunks([1, 2], 3) == [[1, 2]]
assert chunks([1, 2, 3], 0) == []
assert chunks([], 1) == []
```

## `intersperse`

### Behavior and limits

Put `sep` between each pair of elements. An empty list and a singleton are
unchanged: there is nowhere between elements to put anything.

### Signature

```deed
fn intersperse<T>(items: List<T>, sep: T) -> List<T>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `intersperse puts a separator between elements`

```deed
assert intersperse([1, 2, 3], 0) == [1, 0, 2, 0, 3]
assert intersperse(["a", "b"], ",") == ["a", ",", "b"]
assert intersperse([1], 0) == [1]
assert intersperse([], 0) == []
```

## `unzip`

### Behavior and limits

The inverse of `zip`: all lefts together and all rights together.

### Signature

```deed
fn unzip<A, B>(items: List<Pair<A, B>>) -> Sides<A, B>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `unzip splits pairs into two lists`

```deed
let sides = unzip([Pair { left: 1, right: "a" }, Pair { left: 2, right: "b" }])
let empty = unzip([])
let one = unzip([Pair { left: 9, right: true }])
```

## `flat_map`

### Behavior and limits

Map each element to a list and flatten one level. `map` then `flatten`,
written once so a callback that wants to expand or drop does not need two
walks.

### Signature

```deed
fn flat_map<A, B, uses r>(items: List<A>, step: Fn(A) uses r -> List<B>) -> List<B>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `flat_map expands each element into a list`

```deed
assert flat_map([1, 2, 3], |n: Int| [n, n]) == [1, 1, 2, 2, 3, 3]
assert flat_map(
assert flat_map([], |n: Int| [n]) == []
assert flat_map(["a", "bb"], |word: String| [word, word]) == ["a", "a", "bb", "bb"]
```

## `scan`

### Behavior and limits

Every partial fold, in order. `fold` keeps only the last one; scan is for a
caller that wants to see how it got there instead of writing the same loop
with the accumulator swapped for a pair.

### Signature

```deed
fn scan<T, S, uses r>(items: List<T>, start: S, step: Fn(S, T) uses r -> S) -> List<S>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `scan keeps every partial fold, in the order they were made`

```deed
assert scan([1, 2, 3], 0, |total: Int, n: Int| total + n) == [1, 3, 6]
assert scan([], 7, |total: Int, n: Int| total + n) == []
assert scan(["a", "b"], "", |joined: String, word: String| joined + word) == ["a", "ab"]
```

## `transpose`

### Behavior and limits

Rows become columns, stopping at the shortest row rather than inventing a
value for one that ran out.

### Signature

```deed
fn transpose<T>(rows: List<List<T>>) -> List<List<T>>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `transpose turns rows into columns, at the shortest row`

```deed
assert transpose([[1, 2, 3], [4, 5, 6]]) == [[1, 4], [2, 5], [3, 6]]
assert transpose([[1, 2], [3]]) == [[1, 3]]
assert transpose([[1, 2, 3]]) == [[1], [2], [3]]
assert transpose([]) == []
```

## `group_by`

### Behavior and limits

Bucket elements by a key, keeping first-seen order for both the buckets and
what went into them. A group is a `Pair` rather than the table this file
does not import, `left` the key and `right` what landed in it.

### Signature

```deed
fn group_by<T, K, uses r>(items: List<T>, key_of: Fn(T) uses r -> K)
    -> List<Pair<K, List<T>>>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `group_by buckets by key and keeps first-seen order`

```deed
let grouped = group_by([1, 2, 3, 4, 5, 6], |n: Int| n % 2)
assert group_by([], |n: Int| n) == []
```

## `sort`

### Behavior and limits

Insertion sort: each element finds its place among what has already been
placed, using a caller-supplied "does the first belong before the second"
rather than a bound on `T`. `partition` already answers where a new element
splits the list, so placing one is the same three lines as filtering twice.

### Signature

```deed
fn sort<T, uses r>(items: List<T>, before: Fn(T, T) uses r -> Bool) -> List<T>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `sort places each element among what came before it`

```deed
assert sort([3, 1, 2], |a: Int, b: Int| a < b) == [1, 2, 3]
assert sort([3, 1, 2], |a: Int, b: Int| a > b) == [3, 2, 1]
assert sort([1], |a: Int, b: Int| a < b) == [1]
assert sort([], |a: Int, b: Int| a < b) == []
```

## `sum`

### Behavior and limits

The one `fold` was written for.

The comment above `fold` says a library without it stops working the moment
somebody wants a sum, and then this file did not have the sum either. It is
two lines and it saves a closure at every call, which is most of why anybody
reaches for the name.

`Int` rather than `T`, because addition is what is being asked for and this
language has it on `Int` and on `String`, where the answer is `join`.

### Signature

```deed
fn sum(numbers: List<Int>) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/list.deed`

#### `sum adds the list up, and an empty one comes to zero`

```deed
assert sum([1, 2, 3]) == 6
assert sum([0 - 4, 4]) == 0
assert sum([7]) == 7
assert sum([]) == 0
```

## `largest`

### Behavior and limits

The largest, by an order the caller passes.

The same `before` `sort` takes, and for the same reason: `<` is refused on a
type parameter (DEED4020) and a comparator is what this library uses instead
of a bound. Passing the one that sorts a list gives back the one that sort
would have put last.

A `Result` because an empty list has no largest element, which is the answer
`first` and `find` already give for having nothing to hand back.

### Signature

```deed
fn largest<T, uses r>(items: List<T>, before: Fn(T, T) uses r -> Bool)
    -> Result<T, String>
```

### Row variables

`r`

### Declared row

`r`

### Contract

```deed
uses
    r,
```

### Examples from `std/list.deed`

#### `largest reads the order it was handed, and says so about an empty list`

```deed
assert largest([3, 1, 2], |a: Int, b: Int| a < b) == ok(3)
assert largest([3, 1, 2], |a: Int, b: Int| a > b) == ok(1)
assert largest([7], |a: Int, b: Int| a < b) == ok(7)
assert largest(["a", "c", "b"], |a: String, b: String| a < b) == ok("c")
let nothing = largest([], |a: Int, b: Int| a < b)
```
