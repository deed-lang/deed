# Task: audit

Write a module named `bench/audit`.

Declare an effect named `Audit` with two operations:

```
fn note(entry: String) -> ()
fn so_far() -> String
```

Declare a handler named `Collected` that implements it. It keeps every entry it
has been given, and `so_far` gives them back joined with `", "` between them.

Then export:

```
fn record_all(entries: List<String>) -> () uses Audit.note
fn collected(entries: List<String>) -> String
```

`record_all` performs `Audit.note` once per entry, in order.

`collected` installs `Collected`, calls `record_all` inside it, asks it what it
gathered, and gives that back. `collected` itself declares no effects: a `with`
block discharges the effect its handler implements, which is why the function
holding the block does not have to declare it.

A handler's `state` is the only mutable thing in the language, and only a
handler can have it.

Write only the module. No `main`, no tests.
