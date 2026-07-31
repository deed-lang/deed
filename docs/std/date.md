# `std/date`

_Generated from `/std/date.deed` and the module's own tests._

## Module

A calendar on top of `Io.epoch`, which only hands back milliseconds since
1970.

This started as an example, written to find out whether
`design/04-capabilities.md` was still right that the language had no way to
write a calendar. It was not, and the file proved it. What it could not do
was ship: a program elsewhere that wanted a date had to copy it, the same
way `std/list` and `std/table` had to be copied before they moved here.

The conversion is Howard Hinnant's `civil_from_days`: integer arithmetic
only, no lookup table, and no leap-year rule spelled out by name. The rule
falls out of the same division and modulo every other line here uses, which
is why `is_leap_year` below is written separately rather than being the
thing `date_of` consults.

Only forward from 1970. A negative day count, from a clock set before then,
would need floor division where this uses the truncating `/` the language
has, and nothing has needed that yet. `date_of` says so rather than
answering wrongly.

## `days_since_epoch`

### Behavior and limits

How many whole days a millisecond count covers.

### Signature

```deed
fn days_since_epoch(milliseconds: Int) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/date.deed`

#### `a clock set before 1970 is refused rather than answered wrongly`

```deed
assert days_since_epoch(86400000) == 1
assert days_since_epoch(86399999) == 0
```

## `date_of`

### Behavior and limits

The civil date a millisecond count lands on, or a refusal for a clock set
before 1970.

A `Result` rather than a refinement on the parameter: a caller holding a
number out of `Io.epoch` has no way to have proved it is not negative, and
making them prove it would push the check to every call site rather than
removing it.

### Signature

```deed
fn date_of(milliseconds: Int) -> Result<Date, String>
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/date.deed`

#### `a clock set before 1970 is refused rather than answered wrongly`

```deed
match date_of(0) {
```

## `civil_of`

### Behavior and limits

The civil date a day count lands on, counting from 1970-01-01.

Total for days at or after the epoch. Split out from `date_of` because the
arithmetic is about days and the refusal is about milliseconds, and keeping
them apart is what lets `date_of` be the only place that knows the two are
related by a constant.

### Signature

```deed
fn civil_of(days: Int) -> Date
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/date.deed`

#### `the epoch itself is 1970-01-01`

```deed
assert civil_of(0) == Date { year: 1970, month: 1, day: 1 }
```

## `is_leap_year`

### Behavior and limits

Whether a year has a leap day.

Written out rather than derived from `civil_of`, because a caller asking
this is asking about a year rather than about a date, and going through the
conversion to answer would mean inventing a date to ask about.

### Signature

```deed
fn is_leap_year(year: Int) -> Bool
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/date.deed`

#### `the leap year rule is the whole rule`

```deed
assert is_leap_year(2024)
assert !is_leap_year(2023)
assert !is_leap_year(1900)
assert is_leap_year(2000)
```

## `days_in_month`

### Behavior and limits

How many days a month has, in a given year.

A month outside 1 to 12 gives back 0, which is the only answer that does not
require refusing and is what a caller walking a year would read as "there is
no such month".

### Signature

```deed
fn days_in_month(year: Int, month: Int) -> Int
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/date.deed`

#### `months are as long as they are`

```deed
assert days_in_month(2024, 2) == 29
assert days_in_month(2023, 2) == 28
assert days_in_month(2024, 1) == 31
assert days_in_month(2024, 4) == 30
assert days_in_month(2024, 13) == 0
```

## `padded`

### Behavior and limits

A two-digit rendering, for the parts of a date that have two digits.

### Signature

```deed
fn padded(value: Int) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/date.deed`

#### `the text sorts the way the dates do`

```deed
assert padded(9) == "09"
assert padded(10) == "10"
```

## `text`

### Behavior and limits

A date as `YYYY-MM-DD`.

ISO 8601 order, which is the one ordering where sorting the text sorts the
dates. That matters here more than in most languages: `<` on `String` is the
only ordering the language gives text, so a date format that did not have
this property would leave a program unable to sort dates without taking them
apart again.

### Signature

```deed
fn text(date: Date) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/date.deed`

#### `the text sorts the way the dates do`

```deed
assert text(Date { year: 2024, month: 2, day: 9 }) < text(Date { year: 2024, month: 2, day: 10 })
assert text(Date { year: 2024, month: 9, day: 1 }) < text(Date { year: 2024, month: 10, day: 1 })
assert text(Date { year: 1999, month: 12, day: 31 }) < text(Date { year: 2000, month: 1, day: 1 })
```

## `text_of`

### Behavior and limits

The date a millisecond count lands on, as text, or the reason it refused.

### Signature

```deed
fn text_of(milliseconds: Int) -> String
```

### Row variables

`none`

### Declared row

`pure`

### Contract

```deed
pure
```

### Examples from `std/date.deed`

#### `the epoch itself is 1970-01-01`

```deed
assert text_of(0) == "1970-01-01"
```

#### `a well known date, the turn of the millennium's next leap day`

```deed
assert text_of(951868800000) == "2000-03-01"
```

#### `a leap day, and the day right after it`

```deed
assert text_of(1709164800000) == "2024-02-29"
assert text_of(1709251200000) == "2024-03-01"
```

#### `a clock set before 1970 is refused rather than answered wrongly`

```deed
assert text_of(0 - 1) == "this calendar starts at 1970"
```
