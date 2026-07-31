# Task: grade

Write a module named `bench/grade`.

Declare a choice with exactly these three variants, spelled this way:

```
Low
Middling
High
```

Then export:

```
fn grade(score: Int) -> Grade
fn describe(mark: Grade) -> String
```

`grade(score)` gives `Low` below `40`, `High` at `80` and above, and `Middling`
in between.

`describe` gives back `"low"`, `"middling"` or `"high"`.

Name the choice `Grade`.

There is no catch-all pattern in this language: a `match` has to name every
variant, so that adding one later breaks every `match` instead of silently
falling through.

Write only the module. No `main`, no tests.
