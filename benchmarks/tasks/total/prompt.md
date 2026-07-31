# Task: total

Write a module named `bench/total`.

It exports two functions:

```
fn total(numbers: List<Int>) -> Int
fn largest(numbers: List<Int>, fallback: Int) -> Int
```

`total(numbers)` adds every number in the list and gives back the sum. The total
of an empty list is `0`.

`largest(numbers, fallback)` gives back the biggest number in the list, or
`fallback` when the list is empty.

This language has one loop and it walks a list. There is no `while` statement,
no `break` and no `continue`.

Write only the module. No `main`, no tests.
