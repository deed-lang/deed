# Task: split_evenly

Write a module named `bench/split_evenly`.

It exports one function:

```
fn split_evenly(amount: Int, people: Int) -> Result<Int, String>
```

`split_evenly(amount, people)` gives back how much each person gets when
`amount` is shared between `people`.

It gives back an error instead when `people` is zero or negative. The error
message is exactly `"there is nobody to share with"`.

Division truncates towards zero in this language, and that is fine here: the
remainder is not asked for.

`Result` is part of the language. `ok(x)` and `err(x)` build one.

Write only the module. No `main`, no tests.
