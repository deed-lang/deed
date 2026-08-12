# The contracts-free comparison

The six benchmark behaviours were translated to Starlark and handed to the
same `gpt-5.6-luna` model once on 2026-08-12. Starlark was chosen because it is
a Python-like language the model already knows, but its evaluator has no
imports, filesystem, network, process API or ambient host authority. Model code
was run by `go.starlark.net`, not by Python on the benchmark machine.

This is the comparison arm [`README.md`](README.md) asked for: the same visible
behaviour and hidden edge cases in a language with no contracts, refinement
types, effect rows or generated properties.

## Result

| Arm | Runs | Answered | Passed hidden behaviour checks |
| --- | ---: | ---: | ---: |
| Deed with `deed mcp` | 5 | 30/30 | 25/30 |
| Starlark, no tools | 1 | 6/6 | **6/6** |

The Starlark run took 17.0 seconds across all six calls. Every answer was a
single file and all six passed on the first attempt.

This does **not** establish that contracts hurt. It does establish that this
benchmark does not yet show that contracts help: its behaviours are routine in
a familiar general-purpose language, while three Deed tasks spend part of their
prompt budget teaching effects, refinements and `Result`. The comparison is
also one run against five, so it is a direction for the next benchmark rather
than an effect-size estimate.

## The generated answers

```python
# twice.star
def twice(n):
    return n + n
```

```python
# total.star
def total(numbers):
    result = 0
    for number in numbers:
        result += number
    return result

def largest(numbers, fallback):
    if len(numbers) == 0:
        return fallback
    result = numbers[0]
    for number in numbers[1:]:
        if number > result:
            result = number
    return result
```

```python
# grade.star
Low = "low"
Middling = "middling"
High = "high"

def grade(score):
    if score < 40:
        return Low
    if score >= 80:
        return High
    return Middling

def describe(mark):
    if mark == Low:
        return "low"
    if mark == Middling:
        return "middling"
    return "high"
```

```python
# split_evenly.star
def split_evenly(amount, people):
    if people <= 0:
        return (False, "there is nobody to share with")
    if amount < 0:
        return (True, -((-amount) // people))
    return (True, amount // people)
```

```python
# audit.star
def collected(entries):
    return ", ".join(entries)
```

```python
# stock.star
def take_one(count):
    return count - 1

def restock(count, delivered):
    return count + delivered
```

## What was checked

The scorer exercised the same edge cases as the Deed task modules: negative
inputs to `twice`; empty and all-negative lists for `largest`; every grade
boundary; positive, zero and negative divisors; empty, singleton and multiple
audit entries; and restocking followed by taking one.

The scorer was checked in both directions before the model run. The six hand
written references scored 6/6. Replacing `largest`'s first element with the
fallback reproduced the Deed benchmark's surviving bug and scored 5/6, naming
`largest([-5, -2], 0) = 0, want -2`.

## What follows

Do not respond by adding easier Deed tasks or by weakening the comparison. A
second benchmark should ask for work where contracts provide observable value:
a change to one module that breaks an unseen caller, a generated counterexample,
or a proof obligation that distinguishes two implementations which pass the
same examples. Until then, the honest claim is that `deed mcp` makes an unknown
language writable, not that contracts beat a familiar language on these six
functions.
