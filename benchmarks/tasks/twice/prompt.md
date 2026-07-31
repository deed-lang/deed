# Task: twice

Write a module named `bench/twice`.

It exports one function:

```
fn twice(n: Int) -> Int
```

`twice(n)` gives back `n + n`.

Doubling can overflow, and a function in this language may not promise something
it cannot keep. Constrain the input with a `where` clause so the promise holds,
and state the promise with an `ensures` clause saying the result is `n + n`.

Write only the module. No `main`, no tests.
