# `std/map`

_Generated from `/std/map.deed` and the module's own tests._

## Module

A red-black tree, written in Deed.

A self-balancing keyed map, generic in key and value, with a comparator
passed as a function value. The comparator is `Fn(K, K) -> Int`: negative
when the first key is smaller, zero when they are equal, positive when the
first is larger.

Balancing follows Okasaki's "Purely Functional Data Structures". After each
recursive insert the tree calls `balance` (cases where the left child is
red) and `balance_right` (cases where the right child is red) to restore
the red-black invariant. The root is then set to Black by `insert`, which
calls `make_black`.

Walls hit during this implementation, documented as promised:

  FIELD PATTERNS DO NOT FILTER. The interpreter's `matches` function for
  `Pattern::Record` checks only the variant name, not any field sub-
  patterns. Writing `Node { color: Red, ... }` in a match arm does not
  select only Red nodes; it accepts any Node. The field patterns in record
  arms are used only by `bind`, which populates local names. Every color
  check in `balance` and `balance_right` therefore uses `node_color` to
  extract the color field and an explicit nested `match` on the result.
  The classical Okasaki four-case nested field match (used in Haskell and
  Koka) cannot be written in Deed today.

  DIVERGE ON ALL RECURSIVE PATHS. `insert`, `get`, `size`, and `entries`
  are all recursive; each declares `Diverge`. `insert` also calls
  `insert_node`, which carries `Diverge`, so `insert` inherits it even
  though it is not itself recursive.

  NO NATURAL LOOP FOR IN-ORDER WALK. `entries` converts the tree to a list
  by recursion rather than a `for` loop, because `for` walks a list and a
  tree has no built-in iteration order. A list built by `concat` works but
  is not as efficient as an iterative or continuation-based walk would be.

## `node_color`

### Behavior and limits

The color of the root node. An empty tree is considered Black, which is
consistent with the red-black invariant that empty children are Black.

### Signature

```deed
fn node_color<K, V>(map: Map<K, V>) -> Color
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/map.deed`

#### `node_color returns Black for Empty and the stored color for a Node`

```deed
assert node_color(empty) == Black
assert node_color(red) == Red
assert node_color(black) == Black
```

#### `make_black leaves Empty alone and turns a Red root to Black`

```deed
assert node_color(make_black(red)) == Black
```

#### `balance case 1 fixes a black node with a left-leaning red chain`

```deed
assert node_color(result) == Red
```

#### `balance case 2 fixes a black node with an inner left red child`

```deed
assert node_color(result) == Red
```

#### `balance leaves a Red parent alone regardless of children`

```deed
assert node_color(result) == Red
```

#### `balance_right case 3 fixes a black node with a right-leaning inner red chain`

```deed
assert node_color(result) == Red
```

#### `balance_right case 4 fixes a black node with a right-leaning outer red chain`

```deed
assert node_color(result) == Red
```

#### `balance_right leaves a Black right child alone`

```deed
assert node_color(result) == Black
```

#### `insert_node inserts into an empty tree with a Red root`

```deed
assert node_color(t) == Red
```

#### `an inserted key can be retrieved`

```deed
assert node_color(m) == Black
```

#### `the root is always Black after insert`

```deed
assert node_color(m1) == Black
assert node_color(m2) == Black
assert node_color(m3) == Black
```

## `make_black`

### Behavior and limits

The same tree with its root color set to Black. Called by `insert` to
ensure the root is always Black after a full insertion.

### Signature

```deed
fn make_black<K, V>(map: Map<K, V>) -> Map<K, V>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/map.deed`

#### `make_black leaves Empty alone and turns a Red root to Black`

```deed
assert make_black(empty) == Empty
assert node_color(make_black(red)) == Black
assert make_black(black) == black
```

## `balance`

### Behavior and limits

Rebalance a Black node whose left child is Red and may have a Red grandchild.

Checks cases 1 and 2 of Okasaki's four-case balance function:

  Case 1: B (R (R a x b) y c) z d   =>  R (B a x b) y (B c z d)
  Case 2: B (R a x (R b y c)) z d   =>  R (B a x b) y (B c z d)

When neither applies, delegates to `balance_right` for cases 3 and 4.

NOTE: field patterns in match arms do not filter by field value in the
current Deed interpreter. Colors are checked via explicit `match` on the
value returned by `node_color` rather than via field-pattern guards.

### Signature

```deed
fn balance<K, V>(color: Color, left: Map<K, V>, key: K, value: V, right: Map<K, V>)
    -> Map<K, V>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/map.deed`

#### `balance case 1 fixes a black node with a left-leaning red chain`

```deed
let result = balance(Black, left, 3, 30, Empty)
```

#### `balance case 2 fixes a black node with an inner left red child`

```deed
let result = balance(Black, left, 3, 30, Empty)
```

#### `balance leaves a Red parent alone regardless of children`

```deed
let result = balance(Red, left, 2, 20, Empty)
```

## `balance_right`

### Behavior and limits

Rebalance a Black node whose right child is Red and may have a Red grandchild.

Checks cases 3 and 4 of Okasaki's four-case balance function:

  Case 3: B a x (R (R b y c) z d)   =>  R (B a x b) y (B c z d)
  Case 4: B a x (R b y (R c z d))   =>  R (B a x b) y (B c z d)

When neither applies, returns a plain Black node unchanged.

### Signature

```deed
fn balance_right<K, V>(left: Map<K, V>, key: K, value: V, right: Map<K, V>) -> Map<K, V>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/map.deed`

#### `balance_right case 3 fixes a black node with a right-leaning inner red chain`

```deed
let result = balance_right(Empty, 2, 20, right)
```

#### `balance_right case 4 fixes a black node with a right-leaning outer red chain`

```deed
let result = balance_right(Empty, 3, 30, right)
```

#### `balance_right leaves a Black right child alone`

```deed
let result = balance_right(Empty, 3, 30, right)
```

## `insert_node`

### Behavior and limits

Insert a key-value pair into the tree, internal recursive form.

Returns a tree that satisfies all red-black invariants except that the root
may be Red. `insert` wraps this and calls `make_black` to fix the root.

### Signature

```deed
fn insert_node<K, V>(map: Map<K, V>, key: K, value: V, cmp: Fn(K, K) -> Int) -> Map<K, V>
```

### Row variables

`none`

### Declared row

`Diverge`

### Contract

```deed
uses
    Diverge,
```

### Examples from `std/map.deed`

#### `insert_node inserts into an empty tree with a Red root`

```deed
let t = insert_node(Empty, 1, "one", cmp_int)
```

## `insert`

### Behavior and limits

Insert a key-value pair into the map, replacing any existing value for
that key. The root is always Black after this call.

If the key is already there, the value is replaced. Inserting the same key
twice with the same value is the same as inserting it once.

### Signature

```deed
fn insert<K, V>(map: Map<K, V>, key: K, value: V, cmp: Fn(K, K) -> Int) -> Map<K, V>
```

### Row variables

`none`

### Declared row

`Diverge`

### Contract

```deed
uses
    Diverge,
```

### Examples from `std/map.deed`

#### `an inserted key can be retrieved`

```deed
let m = insert(Empty, 1, "one", cmp_int)
```

#### `inserting a key that is there replaces the value`

```deed
let m = insert(insert(Empty, 1, "one", cmp_int), 1, "ONE", cmp_int)
```

#### `multiple keys go in and come back out`

```deed
let m = insert(
insert(insert(Empty, 2, "two", cmp_int), 1, "one", cmp_int),
```

#### `entries come back in sorted order`

```deed
let m = insert(
insert(insert(Empty, 2, "two", cmp_int), 1, "one", cmp_int),
```

#### `the root is always Black after insert`

```deed
let m1 = insert(Empty, 1, "one", cmp_int)
let m2 = insert(m1, 2, "two", cmp_int)
let m3 = insert(m2, 3, "three", cmp_int)
```

#### `inserting keys in ascending order stays balanced`

```deed
let m = insert(
insert(
insert(insert(Empty, 1, "a", cmp_int), 2, "b", cmp_int),
```

#### `string keys work the same way`

```deed
let m = insert(
insert(insert(Empty, "b", 2, cmp_string), "a", 1, cmp_string),
```

#### `size counts each key once even when the same key is inserted twice`

```deed
let m = insert(
insert(insert(Empty, 1, "a", cmp_int), 1, "b", cmp_int),
```

## `get`

### Behavior and limits

What is stored under this key, or an error if there is nothing.

### Signature

```deed
fn get<K, V>(map: Map<K, V>, key: K, cmp: Fn(K, K) -> Int) -> Result<V, String>
```

### Row variables

`none`

### Declared row

`Diverge`

### Contract

```deed
uses
    Diverge,
```

### Examples from `std/map.deed`

#### `balance case 1 fixes a black node with a left-leaning red chain`

```deed
assert get(result, 1, cmp_int) == ok(10)
assert get(result, 2, cmp_int) == ok(20)
assert get(result, 3, cmp_int) == ok(30)
```

#### `balance case 2 fixes a black node with an inner left red child`

```deed
assert get(result, 1, cmp_int) == ok(10)
assert get(result, 2, cmp_int) == ok(20)
assert get(result, 3, cmp_int) == ok(30)
```

#### `balance_right case 3 fixes a black node with a right-leaning inner red chain`

```deed
assert get(result, 2, cmp_int) == ok(20)
assert get(result, 3, cmp_int) == ok(30)
assert get(result, 4, cmp_int) == ok(40)
```

#### `balance_right case 4 fixes a black node with a right-leaning outer red chain`

```deed
assert get(result, 3, cmp_int) == ok(30)
assert get(result, 4, cmp_int) == ok(40)
assert get(result, 5, cmp_int) == ok(50)
```

#### `insert_node inserts into an empty tree with a Red root`

```deed
assert get(t, 1, cmp_int) == ok("one")
```

#### `an empty map holds nothing`

```deed
assert get(m, 1, cmp_int) == err("no such key")
```

#### `an inserted key can be retrieved`

```deed
assert get(m, 1, cmp_int) == ok("one")
assert get(m, 2, cmp_int) == err("no such key")
```

#### `inserting a key that is there replaces the value`

```deed
assert get(m, 1, cmp_int) == ok("ONE")
```

#### `multiple keys go in and come back out`

```deed
assert get(m, 1, cmp_int) == ok("one")
assert get(m, 2, cmp_int) == ok("two")
assert get(m, 3, cmp_int) == ok("three")
assert get(m, 4, cmp_int) == err("no such key")
```

#### `inserting keys in ascending order stays balanced`

```deed
assert get(m, 1, cmp_int) == ok("a")
assert get(m, 3, cmp_int) == ok("c")
assert get(m, 5, cmp_int) == ok("e")
```

#### `string keys work the same way`

```deed
assert get(m, "a", cmp_string) == ok(1)
assert get(m, "b", cmp_string) == ok(2)
assert get(m, "c", cmp_string) == ok(3)
assert get(m, "d", cmp_string) == err("no such key")
```

## `size`

### Behavior and limits

The number of key-value pairs in the map.

### Signature

```deed
fn size<K, V>(map: Map<K, V>) -> Int
```

### Row variables

`none`

### Declared row

`Diverge`

### Contract

```deed
uses
    Diverge,
```

### Examples from `std/map.deed`

#### `an empty map holds nothing`

```deed
assert size(m) == 0
```

#### `an inserted key can be retrieved`

```deed
assert size(m) == 1
```

#### `inserting a key that is there replaces the value`

```deed
assert size(m) == 1
```

#### `multiple keys go in and come back out`

```deed
assert size(m) == 3
```

#### `inserting keys in ascending order stays balanced`

```deed
assert size(m) == 5
```

#### `size counts each key once even when the same key is inserted twice`

```deed
assert size(m) == 2
```

## `entries`

### Behavior and limits

All key-value pairs, in the order the comparator puts them (ascending).

In-order traversal of a red-black tree visits the left subtree, then the
root, then the right subtree, which produces a sorted sequence when the
tree was built with a consistent comparator.

There is no natural `for`-loop walk over a tree. This function builds the
sorted list by recursion and concatenation, which is correct but less
efficient than an iterative approach would be.

### Signature

```deed
fn entries<K, V>(map: Map<K, V>) -> List<Entry<K, V>>
```

### Row variables

`none`

### Declared row

`Diverge`

### Contract

```deed
uses
    Diverge,
```

### Examples from `std/map.deed`

#### `an empty map holds nothing`

```deed
assert entries(m) == []
```

#### `entries come back in sorted order`

```deed
let es = entries(m)
```

#### `inserting keys in ascending order stays balanced`

```deed
let es = entries(m)
```

#### `string keys work the same way`

```deed
let es = entries(m)
```

## `cmp_int`

### Behavior and limits

A comparator for integers. Negative when a < b, zero when equal, positive
when a > b. Pass to `insert` and `get` for an integer-keyed map.

### Signature

```deed
fn cmp_int(a: Int, b: Int) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/map.deed`

#### `balance case 1 fixes a black node with a left-leaning red chain`

```deed
assert get(result, 1, cmp_int) == ok(10)
assert get(result, 2, cmp_int) == ok(20)
assert get(result, 3, cmp_int) == ok(30)
```

#### `balance case 2 fixes a black node with an inner left red child`

```deed
assert get(result, 1, cmp_int) == ok(10)
assert get(result, 2, cmp_int) == ok(20)
assert get(result, 3, cmp_int) == ok(30)
```

#### `balance_right case 3 fixes a black node with a right-leaning inner red chain`

```deed
assert get(result, 2, cmp_int) == ok(20)
assert get(result, 3, cmp_int) == ok(30)
assert get(result, 4, cmp_int) == ok(40)
```

#### `balance_right case 4 fixes a black node with a right-leaning outer red chain`

```deed
assert get(result, 3, cmp_int) == ok(30)
assert get(result, 4, cmp_int) == ok(40)
assert get(result, 5, cmp_int) == ok(50)
```

#### `insert_node inserts into an empty tree with a Red root`

```deed
let t = insert_node(Empty, 1, "one", cmp_int)
assert get(t, 1, cmp_int) == ok("one")
```

#### `an empty map holds nothing`

```deed
assert get(m, 1, cmp_int) == err("no such key")
```

#### `an inserted key can be retrieved`

```deed
let m = insert(Empty, 1, "one", cmp_int)
assert get(m, 1, cmp_int) == ok("one")
assert get(m, 2, cmp_int) == err("no such key")
```

#### `inserting a key that is there replaces the value`

```deed
let m = insert(insert(Empty, 1, "one", cmp_int), 1, "ONE", cmp_int)
assert get(m, 1, cmp_int) == ok("ONE")
```

#### `multiple keys go in and come back out`

```deed
insert(insert(Empty, 2, "two", cmp_int), 1, "one", cmp_int),
cmp_int,
assert get(m, 1, cmp_int) == ok("one")
assert get(m, 2, cmp_int) == ok("two")
assert get(m, 3, cmp_int) == ok("three")
assert get(m, 4, cmp_int) == err("no such key")
```

#### `entries come back in sorted order`

```deed
insert(insert(Empty, 2, "two", cmp_int), 1, "one", cmp_int),
cmp_int,
```

#### `the root is always Black after insert`

```deed
let m1 = insert(Empty, 1, "one", cmp_int)
let m2 = insert(m1, 2, "two", cmp_int)
let m3 = insert(m2, 3, "three", cmp_int)
```

#### `inserting keys in ascending order stays balanced`

```deed
insert(insert(Empty, 1, "a", cmp_int), 2, "b", cmp_int),
cmp_int,
assert get(m, 1, cmp_int) == ok("a")
assert get(m, 3, cmp_int) == ok("c")
assert get(m, 5, cmp_int) == ok("e")
```

#### `size counts each key once even when the same key is inserted twice`

```deed
insert(insert(Empty, 1, "a", cmp_int), 1, "b", cmp_int),
cmp_int,
```

#### `cmp_int gives negative for less, zero for equal, positive for greater`

```deed
assert cmp_int(1, 2) < 0
assert cmp_int(2, 2) == 0
assert cmp_int(3, 2) > 0
```

## `cmp_string`

### Behavior and limits

A comparator for strings, by the same three-way convention.

### Signature

```deed
fn cmp_string(a: String, b: String) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/map.deed`

#### `string keys work the same way`

```deed
insert(insert(Empty, "b", 2, cmp_string), "a", 1, cmp_string),
cmp_string,
assert get(m, "a", cmp_string) == ok(1)
assert get(m, "b", cmp_string) == ok(2)
assert get(m, "c", cmp_string) == ok(3)
assert get(m, "d", cmp_string) == err("no such key")
```

#### `cmp_string gives negative for less, zero for equal, positive for greater`

```deed
assert cmp_string("a", "b") < 0
assert cmp_string("b", "b") == 0
assert cmp_string("c", "b") > 0
```
