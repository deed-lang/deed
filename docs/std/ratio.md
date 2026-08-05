# `std/ratio`

_Generated from `/std/ratio.deed` and the module's own tests._

## Module

Exact ratios, written in Deed.

`design/fractional-values.md` refused every fractional number type and said
what would change the answer: a real program that has to store, compare and
pass fractional quantities around as values rather than only print them. It
also named the honest first move, which is a library rather than a language
change.

This is that library. Nothing here is compiler machinery: a `Ratio` is two
`Int`s in a record, and every operation on it is an ordinary function. What
it demonstrates is that the refusal costs a program less than it looks like,
and where exactly it still costs something.

Two things are load-bearing and neither is decoration.

Equality in this language is structural and total, so `1/2` and `2/4` would
compare unequal unless every `Ratio` that exists is already in lowest terms
with the sign in one place. `simplified` is what makes that true, and every
constructor here goes through it. That is why there is no way to build a
`Ratio` by writing the record literal yourself and having it behave: you can,
and then `==` tells you something you did not mean.

The greatest common divisor is a loop rather than a recursive function on
purpose. Written recursively it would declare `Diverge`, and `Diverge` spreads
to everything that calls it, so a report that wants one percentage would end
up unable to promise anything about termination. Euclid's algorithm is bounded
by the Fibonacci numbers below `Int`'s ceiling, which is ninety-something
steps, so a bounded walk is not a workaround here. It is the honest shape.

## `euclid_steps`

### Behavior and limits

Enough turns for any pair of `Int`s.

Euclid's worst case is consecutive Fibonacci numbers, and the largest one
below `Int`'s ceiling is the ninety-first, so ninety-two steps settle every
input. The extra few are slack rather than a guess.

### Signature

```deed
fn euclid_steps() -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `the divisor walk settles for numbers that need the most steps`

```deed
assert euclid_steps() > 92
```

## `absolute`

### Behavior and limits

The size of a number, ignoring its sign.

The smallest `Int` has no positive counterpart, so `0 - n` overflows for it
and there is no answer to give. That is a precondition rather than a
`Result`: every caller here reaches this through a constructor that has
already turned the number away, and a `Result` would make each of them
carry a failure none of them can have.

### Signature

```deed
fn absolute(n: Int) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
where
    n > Int.min,
  ensures
    ok  => result >= 0,
```

### Examples from `std/ratio.deed`

#### `zero and sign read the way they are written`

```deed
assert absolute(0 - 7) == 7
assert absolute(7) == 7
assert absolute(0) == 0
```

## `greatest_common_divisor`

### Behavior and limits

The greatest common divisor of two numbers, ignoring their signs.

`gcd(0, 0)` is `0`, which is the usual convention and the only answer that
does not require refusing. Nothing here divides by it: `simplified` checks
for zero first.

### Signature

```deed
fn greatest_common_divisor(a: Int, b: Int) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
where
    a > Int.min,
    b > Int.min,
  ensures
    ok  => result >= 0,
```

### Examples from `std/ratio.deed`

#### `the divisor walk settles for numbers that need the most steps`

```deed
assert greatest_common_divisor(6765, 4181) == 1
assert greatest_common_divisor(0, 5) == 5
assert greatest_common_divisor(0, 0) == 0
assert greatest_common_divisor(0 - 12, 18) == 6
```

## `simplified`

### Behavior and limits

The canonical form: lowest terms, sign on top, `bottom` positive.

Called by every constructor. `bottom` of zero is not this function's problem
to refuse, because it has no way to say so; `ratio` refuses it before getting
here, and the two callers that skip `ratio` build their own denominators.

### Signature

```deed
fn simplified(top: Int, bottom: Int) -> Ratio
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
where
    top > Int.min,
    bottom > Int.min,
    bottom != 0,
  ensures
    ok  => result.bottom > 0,
```

### Examples from `std/ratio.deed`

#### `arithmetic stays exact`

```deed
let third = simplified(1, 3)
let sixth = simplified(1, 6)
assert added(third, sixth) == simplified(1, 2)
assert multiplied(third, sixth) == simplified(1, 18)
```

#### `a third of a third of a third is still exact`

```deed
let third = simplified(1, 3)
```

#### `ratios can be ordered`

```deed
let third = simplified(1, 3)
let half = simplified(1, 2)
let minus_half = simplified(0 - 1, 2)
```

#### `the whole part truncates towards zero`

```deed
assert truncated(simplified(7, 2)) == 3
assert truncated(simplified(0 - 7, 2)) == 0 - 3
```

#### `scaling rounds half away from zero, which truncation would not`

```deed
assert scaled(simplified(7, 2), 1) == 4
assert scaled(simplified(0 - 7, 2), 1) == 0 - 4
assert scaled(simplified(2, 3), 100) == 67
```

#### `rendering rounds half away from zero`

```deed
let two_thirds = simplified(2, 3)
assert text(simplified(1, 2), 0) == "1"
assert text(simplified(0 - 1, 2), 0) == "-1"
assert text(simplified(0 - 2, 3), 3) == "-0.667"
```

#### `a rendering keeps the places it was asked for`

```deed
assert text(simplified(1, 8), 4) == "0.1250"
assert text(simplified(1, 3), 0 - 2) == "0"
```

#### `zero and sign read the way they are written`

```deed
assert is_zero(simplified(0, 5))
assert is_negative(simplified(0 - 1, 2))
assert negated(simplified(1, 2)) == simplified(0 - 1, 2)
assert negated(negated(simplified(1, 2))) == simplified(1, 2)
```

## `ratio`

### Behavior and limits

A ratio of two numbers, or a refusal when the second is zero or either is
the smallest `Int`.

A `Result` rather than a refinement on the parameters: a denominator is
usually a count that came from somewhere, and the caller is the one who knows
whether zero means "no data" or "a bug". The smallest `Int` is turned away
here for a different reason, and it is the door that does it: `simplified`
takes the size of both numbers and the smallest `Int` has no positive
counterpart, so this is where the check happens once and everything below
gets to be proven rather than guarded.
Written as `<=` rather than `==` on purpose. The two say the same thing
about an `Int`, since nothing is below the smallest one, but a comparison is
what narrows a range: after `top <= Int.min` fails, `top > Int.min` is a
fact, and after `top == Int.min` fails all the checker knows is that one
number is out. That is the difference between the call below being proven
and being checked again at run time.

### Signature

```deed
fn ratio(top: Int, bottom: Int) -> Result<Ratio, String>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `a ratio is stored in lowest terms`

```deed
let half = value_or(ratio(2, 4), zero())
```

#### `two ways of writing the same ratio are the same value`

```deed
assert ratio(2, 4) == ratio(1, 2)
assert ratio(1, 0 - 2) == ratio(0 - 1, 2)
assert ratio(3, 9) == ratio(1, 3)
```

#### `a denominator of zero is refused rather than guessed at`

```deed
let why = reason_or(ratio(1, 0), "it did not refuse")
```

## `whole`

### Behavior and limits

A whole number as a ratio. Total, because every `Int` is one.

### Signature

```deed
fn whole(n: Int) -> Ratio
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `arithmetic stays exact`

```deed
assert added(third, added(third, third)) == whole(1)
```

#### `a third of a third of a third is still exact`

```deed
assert value_or(divided(whole(1), small), zero()) == whole(27)
```

#### `dividing by zero is refused`

```deed
let why = reason_or(divided(whole(1), zero()), "it did not refuse")
```

#### `the whole part truncates towards zero`

```deed
assert truncated(whole(4)) == 4
```

#### `scaling rounds half away from zero, which truncation would not`

```deed
assert scaled(whole(5), 3) == 15
```

#### `a rendering keeps the places it was asked for`

```deed
assert text(whole(3), 2) == "3.00"
```

#### `zero and sign read the way they are written`

```deed
assert !is_zero(whole(1))
```

## `zero`

### Behavior and limits

The ratio nothing is a share of, and the identity `added` leaves alone.

### Signature

```deed
fn zero() -> Ratio
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `a ratio is stored in lowest terms`

```deed
let half = value_or(ratio(2, 4), zero())
```

#### `a third of a third of a third is still exact`

```deed
assert value_or(divided(whole(1), small), zero()) == whole(27)
```

#### `dividing by zero is refused`

```deed
let why = reason_or(divided(whole(1), zero()), "it did not refuse")
```

#### `a rendering keeps the places it was asked for`

```deed
assert text(zero(), 2) == "0.00"
```

#### `zero and sign read the way they are written`

```deed
assert is_zero(zero())
assert !is_negative(zero())
```

## `is_zero`

### Behavior and limits

Whether this is that one. Reads the numerator, because a canonical zero has
a denominator of one and no other form of zero exists.

### Signature

```deed
fn is_zero(value: Ratio) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `zero and sign read the way they are written`

```deed
assert is_zero(zero())
assert is_zero(simplified(0, 5))
assert !is_zero(whole(1))
```

## `is_negative`

### Behavior and limits

Whether this is below zero. The sign lives on top by construction, so there
is only one place to look.

### Signature

```deed
fn is_negative(value: Ratio) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `zero and sign read the way they are written`

```deed
assert is_negative(simplified(0 - 1, 2))
assert !is_negative(zero())
```

## `negated`

### Behavior and limits

The same distance from zero, the other way.

### Signature

```deed
fn negated(value: Ratio) -> Ratio
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `zero and sign read the way they are written`

```deed
assert negated(simplified(1, 2)) == simplified(0 - 1, 2)
assert negated(negated(simplified(1, 2))) == simplified(1, 2)
```

## `added`

### Behavior and limits

The sum, in lowest terms.

Over the common denominator rather than the least one: the least common
multiple would keep the numbers smaller, and `simplified` brings them back
down anyway, so it would buy a smaller intermediate at the cost of a second
divisor walk.

### Signature

```deed
fn added(left: Ratio, right: Ratio) -> Ratio
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `arithmetic stays exact`

```deed
assert added(third, sixth) == simplified(1, 2)
assert added(third, added(third, third)) == whole(1)
```

## `subtracted`

### Behavior and limits

The difference, which is the sum with one side turned around.

### Signature

```deed
fn subtracted(left: Ratio, right: Ratio) -> Ratio
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `arithmetic stays exact`

```deed
assert subtracted(third, sixth) == sixth
```

## `multiplied`

### Behavior and limits

The product, in lowest terms.

### Signature

```deed
fn multiplied(left: Ratio, right: Ratio) -> Ratio
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `arithmetic stays exact`

```deed
assert multiplied(third, sixth) == simplified(1, 18)
```

#### `a third of a third of a third is still exact`

```deed
let small = multiplied(multiplied(third, third), third)
```

## `divided`

### Behavior and limits

Division, refused when the divisor is zero.

The same shape as `ratio` and for the same reason: dividing by nothing is a
question about the caller's data, not about arithmetic.

### Signature

```deed
fn divided(left: Ratio, right: Ratio) -> Result<Ratio, String>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `a third of a third of a third is still exact`

```deed
assert value_or(divided(whole(1), small), zero()) == whole(27)
```

#### `dividing by zero is refused`

```deed
let why = reason_or(divided(whole(1), zero()), "it did not refuse")
```

## `is_below`

### Behavior and limits

Whether `left` is below `right`.

Cross multiplication, which is exact because both denominators are positive
by construction.

### Signature

```deed
fn is_below(left: Ratio, right: Ratio) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `ratios can be ordered`

```deed
assert is_below(third, half)
assert !is_below(half, third)
assert !is_below(half, half)
assert is_below(minus_half, third)
```

## `is_above`

### Behavior and limits

Whether `left` is above `right`, which is the same question the other way.

### Signature

```deed
fn is_above(left: Ratio, right: Ratio) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `ratios can be ordered`

```deed
assert is_above(half, minus_half)
```

## `smaller`

### Behavior and limits

The smaller of two ratios.

### Signature

```deed
fn smaller(left: Ratio, right: Ratio) -> Ratio
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `ratios can be ordered`

```deed
assert smaller(third, half) == third
```

## `larger`

### Behavior and limits

The larger of two ratios.

### Signature

```deed
fn larger(left: Ratio, right: Ratio) -> Ratio
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `ratios can be ordered`

```deed
assert larger(third, half) == half
```

## `truncated`

### Behavior and limits

The whole part, truncated towards zero, which is what `/` already does.

### Signature

```deed
fn truncated(value: Ratio) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `the whole part truncates towards zero`

```deed
assert truncated(simplified(7, 2)) == 3
assert truncated(simplified(0 - 7, 2)) == 0 - 3
assert truncated(whole(4)) == 4
```

## `scaled`

### Behavior and limits

`value` multiplied by `factor`, rounded half away from zero.

The one place a rounding policy is chosen, and it is chosen here rather than
inside `text` so that a caller who wants a different one can build on this
instead of reimplementing the ratio.

### Signature

```deed
fn scaled(value: Ratio, factor: Int) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `scaling rounds half away from zero, which truncation would not`

```deed
assert scaled(simplified(7, 2), 1) == 4
assert scaled(simplified(0 - 7, 2), 1) == 0 - 4
assert scaled(simplified(2, 3), 100) == 67
assert scaled(whole(5), 3) == 15
```

## `power_of_ten`

### Behavior and limits

Ten to the power of `digits`, for a decimal rendering to scale by.

A walk rather than a recursive function, for the reason
`greatest_common_divisor` is one: recursion here would declare `Diverge` and
spread it to every caller that wanted a percentage.
Ten to the power of `digits`, for a decimal rendering to scale by.

A walk rather than a recursive function, for the reason
`greatest_common_divisor` is one: recursion here would declare `Diverge` and
spread it to every caller that wanted a percentage.

### Signature

```deed
fn power_of_ten(digits: Int) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `scaling rounds half away from zero, which truncation would not`

```deed
assert power_of_ten(0) == 1
assert power_of_ten(3) == 1000
```

## `zero_padded`

### Behavior and limits

`text` left padded with zeros to `width`.

`std/string` has `pad_left`, but it pads with spaces, and the padding a
decimal place needs is zeros. Written here rather than widening that one,
because a second parameter there would change every existing call.

### Signature

```deed
fn zero_padded(shown: String, width: Int) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `scaling rounds half away from zero, which truncation would not`

```deed
assert zero_padded("7", 3) == "007"
assert zero_padded("1234", 2) == "1234"
```

## `text`

### Behavior and limits

A decimal rendering with exactly `digits` places after the point.

Rounded rather than truncated, half away from zero, and that is a decision
this function makes on the caller's behalf rather than a property of the
value: `2/3` has no finite decimal expansion, so somebody has to choose. The
value itself stays exact until this is called, which is the whole reason a
ratio is worth having over a float.

A negative digit count is read as zero, on the same reasoning `repeat` uses
for a negative count: the caller asked for no places, not for an error.

### Signature

```deed
fn text(value: Ratio, digits: Int) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `rendering rounds half away from zero`

```deed
assert text(two_thirds, 0) == "1"
assert text(two_thirds, 1) == "0.7"
assert text(two_thirds, 3) == "0.667"
assert text(simplified(1, 2), 0) == "1"
assert text(simplified(0 - 1, 2), 0) == "-1"
assert text(simplified(0 - 2, 3), 3) == "-0.667"
```

#### `a rendering keeps the places it was asked for`

```deed
assert text(simplified(1, 8), 4) == "0.1250"
assert text(whole(3), 2) == "3.00"
assert text(zero(), 2) == "0.00"
assert text(simplified(1, 3), 0 - 2) == "0"
```

## `percent_text`

### Behavior and limits

`count` out of `total`, as a percentage with `digits` places.

The call `design/fractional-values.md` said a report would want first. It is
two integers in and text out, but the middle of it is a value: the percentage
exists as an exact ratio, gets compared or carried if the caller wants, and
only becomes text here.

### Signature

```deed
fn percent_text(count: Int, total: Int, digits: Int) -> Result<String, String>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `a percentage is a ratio that became text at the end`

```deed
assert percent_or(percent_text(2, 3, 1), "it refused") == "66.7"
assert percent_or(percent_text(1, 200, 1), "it refused") == "0.5"
assert percent_or(percent_text(1, 4, 0), "it refused") == "25"
let why = percent_reason_or(percent_text(1, 0, 1), "it did not refuse")
```

## `value_or`

### Behavior and limits

The value a fallible call produced, or `fallback` when it refused.

Here rather than in a test because a caller that has already established the
denominator is not zero should not have to write a `match` that cannot take
its second arm. `percent_or` is the same shape for the text form.

### Signature

```deed
fn value_or(result: Result<Ratio, String>, fallback: Ratio) -> Ratio
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `a ratio is stored in lowest terms`

```deed
let half = value_or(ratio(2, 4), zero())
```

#### `a third of a third of a third is still exact`

```deed
assert value_or(divided(whole(1), small), zero()) == whole(27)
```

## `reason_or`

### Behavior and limits

Why a fallible call refused, or `fallback` when it did not.

### Signature

```deed
fn reason_or(result: Result<Ratio, String>, fallback: String) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `a denominator of zero is refused rather than guessed at`

```deed
let why = reason_or(ratio(1, 0), "it did not refuse")
```

#### `dividing by zero is refused`

```deed
let why = reason_or(divided(whole(1), zero()), "it did not refuse")
```

## `percent_or`

### Behavior and limits

The text a percentage call produced, or `fallback` when it refused.

### Signature

```deed
fn percent_or(result: Result<String, String>, fallback: String) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `a percentage is a ratio that became text at the end`

```deed
assert percent_or(percent_text(2, 3, 1), "it refused") == "66.7"
assert percent_or(percent_text(1, 200, 1), "it refused") == "0.5"
assert percent_or(percent_text(1, 4, 0), "it refused") == "25"
```

## `percent_reason_or`

### Behavior and limits

Why a percentage call refused, or `fallback` when it did not.

### Signature

```deed
fn percent_reason_or(result: Result<String, String>, fallback: String) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/ratio.deed`

#### `a percentage is a ratio that became text at the end`

```deed
let why = percent_reason_or(percent_text(1, 0, 1), "it did not refuse")
```
