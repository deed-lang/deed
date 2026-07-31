# `std/string`

_Generated from `/std/string.deed` and the module's own tests._

## Module

The string operations that are missing from the prelude on purpose.

design/02-syntax.md has said for a long time that slicing, searching, case
and padding are missing and that they want a standard library. All four are
here. They are not in the prelude because of the rule that decides what
goes there: a thing that can be written in Deed is written in Deed, and
every function below is. `trim` is in the prelude because it cannot be, and
`contains` is not because it can.

Case (`to_upper`, `to_lower`, #672) is a table, the twenty-six ASCII
letters and nothing else, the same limit `design/02-syntax.md` states for
the reason a name this short may not carry a locale table a reader of the
signature cannot see. A character outside that table comes back exactly as
it went in, which is a promise this file keeps on purpose: text in a
script that has no case, or a script whose case rules do not fit a
one-character-to-one-character table (German `ß`, Turkish dotted and
dotless `i`, anything where one character becomes two), is left alone
rather than silently mangled by a rule that was not written for it. A
program that needs the real definition needs a name of its own, the same
answer this file gives about Unicode whitespace.

Everything walks. `slice` is a fold over the characters, `index_of` is a
`split`, and a program that holds enough text for that to matter wants a
different representation rather than a different language. That is the same
answer `std/table` gives about lookups.

## `slice`

### Behavior and limits

The characters from `from` up to but not including `to`.

Out of range rather than an error, on both ends: asking for more than is
there is answered by what is there. An index is a position between
characters, so `slice(text, 0, 0)` and `slice(text, n, n)` are both empty and
neither is a mistake.

### Signature

```deed
fn slice(text: String, from: Int, to: Int) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `a slice is the characters between two positions`

```deed
assert slice("abcdef", 1, 4) == "bcd"
assert slice("abcdef", 0, 6) == "abcdef"
assert slice("abc", 2, 2) == ""
```

#### `a slice asking for more than is there gets what is there`

```deed
assert slice("abc", 0, 99) == "abc"
assert slice("abc", 0 - 5, 2) == "ab"
assert slice("abc", 99, 200) == ""
```

## `index_of`

### Behavior and limits

Where `needle` starts, or the length of `text` when it is not there.

The length rather than a `Result`, because every caller of this compares the
answer with something, and the position one past the end compares the way a
missing needle should: no slice starts there and no prefix reaches it.

### Signature

```deed
fn index_of(text: String, needle: String) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `a needle that is not there answers past the end`

```deed
assert index_of("hello world", "world") == 6
assert index_of("hello world", "hello") == 0
assert index_of("hello", "z") == 5
```

## `starts_with`

### Behavior and limits

Whether `text` begins with `prefix`.

An empty prefix matches everything. A prefix longer than the text does not.

### Signature

```deed
fn starts_with(text: String, prefix: String) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `the ends`

```deed
assert starts_with("hello", "he") == true
assert starts_with("hello", "lo") == false
assert starts_with("hello", "") == true
```

## `ends_with`

### Behavior and limits

Whether `text` ends with `suffix`.

An empty suffix matches everything. A suffix longer than the text does not.

### Signature

```deed
fn ends_with(text: String, suffix: String) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `the ends`

```deed
assert ends_with("hello", "lo") == true
assert ends_with("hello", "he") == false
```

## `contains`

### Behavior and limits

Whether `needle` appears anywhere in `text`.

One comparison on a split, which is what design/02-syntax.md already names
as the reason this is not a prelude function: it can be written here.

### Signature

```deed
fn contains(text: String, needle: String) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `contains is whether a split found a cut`

```deed
assert contains("hello world", "world") == true
assert contains("hello world", "planet") == false
assert contains("aaa", "aa") == true
assert contains("", "x") == false
```

## `replace`

### Behavior and limits

Every `from` replaced by `to`. Split and join are already inverse on the
pieces they make, so this is the same walk written once.

### Signature

```deed
fn replace(text: String, from: String, to: String) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `replace puts the new piece where the old one was`

```deed
assert replace("a-b-c", "-", "/") == "a/b/c"
assert replace("hello", "l", "L") == "heLLo"
assert replace("hello", "z", "Z") == "hello"
assert replace("", "a", "b") == ""
```

## `pad_left`

### Behavior and limits

`text` in a column `width` wide, or `text` when it is already wider.

Wider is not an error. The call that wants this is building a report, and a
report with one long name in it should come out crooked rather than not come
out. `repeat` already answers a negative count with an empty list, which is
the same decision one layer down.

These two are written in the order design/02-syntax.md reads them out, and
the check on that sentence holds the order rather than the membership. A
paragraph that lists what a module has is read down the page against the
module, so the two orders being the same is the whole of what makes it
readable; a set comparison passes on a sentence nobody could follow.

### Signature

```deed
fn pad_left(text: String, width: Int) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `padding fills to a width and never cuts`

```deed
assert pad_left("ab", 5) == "   ab"
assert pad_left("abcdef", 3) == "abcdef"
```

## `pad_right`

### Behavior and limits

The same padding, on the right instead of the left.

Wider is still quiet: text already past `width` is returned unchanged.

### Signature

```deed
fn pad_right(text: String, width: Int) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `padding fills to a width and never cuts`

```deed
assert pad_right("ab", 5) == "ab   "
assert pad_right("abcdef", 3) == "abcdef"
```

## `trim_start`

### Behavior and limits

`text` with the whitespace at the front gone.

The whitespace table is the same four characters `trim` uses: space, tab,
carriage return and line feed, not Unicode whitespace.

### Signature

```deed
fn trim_start(text: String) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `trim_start and trim_end each take one side`

```deed
assert trim_start("  hi  ") == "hi  "
assert trim_start("hi") == "hi"
assert trim_start("   ") == ""
assert trim_start("") == ""
```

## `trim_end`

### Behavior and limits

The same, from the other end. One forward pass: the run resets at every
non-space character, so what it holds at the end is exactly the run
touching the end of the string.

### Signature

```deed
fn trim_end(text: String) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `trim_start and trim_end each take one side`

```deed
assert trim_end("  hi  ") == "  hi"
assert trim_end("hi") == "hi"
assert trim_end("   ") == ""
assert trim_end("") == ""
```

## `to_upper`

### Behavior and limits

Case conversion is a character-code table, not a rewrite: what a character
becomes depends on what it is, not on anything the walk that reads it
knows. `index_of` on the two alphabets below is that table, written the
only way this language can write one today.

The table is exactly `a`-`z` and `A`-`Z`. A character not in it, accented,
non-Latin, or anything else, comes back unchanged rather than guessed at:
this is the honest limit of a table with twenty-six entries, stated rather
than left for a caller to discover (#672).

### Signature

```deed
fn to_upper(text: String) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `to_upper and to_lower touch letters and leave everything else`

```deed
assert to_upper("Hello, World! 123") == "HELLO, WORLD! 123"
assert to_upper("") == ""
```

#### `a character outside a-z/A-Z passes through to_upper and to_lower unchanged`

```deed
assert to_upper("café") == "CAFé"
```

## `to_lower`

### Behavior and limits

The same table as `to_upper`, the other direction. Same limit: a character
outside `a`-`z`/`A`-`Z` is returned as it was given.

### Signature

```deed
fn to_lower(text: String) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/string.deed`

#### `to_upper and to_lower touch letters and leave everything else`

```deed
assert to_lower("Hello, World! 123") == "hello, world! 123"
assert to_lower("ALREADY") == "already"
```

#### `a character outside a-z/A-Z passes through to_upper and to_lower unchanged`

```deed
assert to_lower("CAFé") == "café"
```
