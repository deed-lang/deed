# Decision: a spawned task's effect row remains its own

- Status: Proposed
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

Part of the concurrency epic (deed-lang/deed#575), specifically
deed-lang/deed#607.

Every other language that has concurrent tasks hides what those tasks do.
`go f()` says nothing. `tokio::spawn` says nothing. In Deed the row is
already part of the function type, so `spawn worker` where `worker` has
row `uses Io.read, Log.note` already tells a reader everything a spawned
`worker` may do. The static effect checker already enforces it. The
runtime row check (`DEED6010`) already enforces it again on every active
call.

What has to be worked out is what happens to that enforcement when a task
is suspended and resumed under a *different* set of handlers than the one
it started under. `deed-lang/deed#137` solved the related problem for
handler operations: effects performed inside a handler's body are charged
to the `with` block that installed the handler, not to the frames in
between, using a barrier inside `check_row`. A suspended task resumed
elsewhere is the same question with the frames separated in time rather
than in space.

Spawn does not parse yet. This record documents the semantics that must
hold when it does, and the exact mechanism the interpreter will need.

## Decision

**The row of a spawned task is the row of the function being spawned.**
The spawn call site adds nothing to it and removes nothing from it. If
`worker` is typed `uses Io.read, Log.note -> Report` then any `spawn
worker(...)` expression produces a task that may use exactly `Io.read`
and `Log.note`, regardless of what handlers the spawner or the scheduler
have installed.

**A task cannot gain a capability by being resumed somewhere else.** The
handler context at the resume site is invisible to the task's own row
frames. Only handlers installed *inside* the task (by a `with` block
inside the task's body) discharge effects that the task performs.

**Spawning is an effect in the spawner's row.** A function that spawns
`worker` is causing `Io.read` and `Log.note` to happen eventually. The
exact form of that entry depends on the spawn surface, which is an open
question; one natural spelling is that the spawner's row must cover every
operation in the spawned function's row, the same way calling a function
directly requires the caller to cover its effects.

**The runtime check must survive suspension and resumption.** This
requires frame isolation: a task's `rows` and `handlers` state is sealed
at spawn time, and when the task is resumed, a barrier separates its
frames from the handlers installed by whoever resumed it. Concretely, the
`handled` field of every `RowFrame` inside a task is an offset from the
task's own handler base, not from the global handler stack. On resume,
that base is added back so the barrier arithmetic stays correct.

## Drawbacks (required)

Requiring the spawner's row to list every effect the spawned task may
perform makes spawn sites verbose for tasks with wide rows. A function
that coordinates many workers could accumulate a long row just from the
spawns.

The frame-isolation mechanism adds complexity to the interpreter's task
scheduler. Saving and restoring the `rows` and `handlers` context on
every suspend and resume has a cost proportional to the task's call depth.

## Rejected Ideas (required)

- Option: a spawned task inherits the handler context of whoever resumes it.
  - Rejected because: the task's row would then be meaningless. A task
    that declared `uses Log.note` could silently perform any effect a
    lucky scheduling choice happened to have a handler for. The row stops
    being a contract and becomes a hint.

- Option: spawning is not in the spawner's row because the task runs
  independently.
  - Rejected because: "independently" is an implementation detail, not a
    capability argument. The task still performs those effects in the
    world, and whoever decided to spawn it is the cause. Letting the row
    fall silent about that is the same move that led to `go f()` saying
    nothing.

- Option: adopt a separate `Task<Row>` type that carries its row as a
  type parameter and leave the spawner's row blank.
  - Rejected because: a type parameter threads the row through the type
    system but does not charge it to anyone. The spawner still performed
    the spawn, and a row that charges it nothing is a row that says
    nothing about a real choice the spawner made.

## Open Questions (required)

- What does `spawn` look like syntactically? The row of the spawned
  function is already visible in its type, so no annotation is needed at
  the call site, but the keyword and the form of the result value (handle,
  future, channel endpoint) are not settled.

- When the spawner's row must cover the spawned task's row, does that
  mean listing every operation, or naming the effects, or some union
  operation the type system computes? The same question arises for
  higher-order functions and is already answered by the row-variable
  mechanism; the spawn case may want the same answer.

- How does a runtime handler that was installed before the spawn interplay
  with the task? If the spawner writes `with Log { spawn worker() }`, the
  handler is outside the task boundary. The task's row includes `Log.note`
  and the handler is available at the resume site. Whether the barrier
  should let that through (because the `with` preceded the spawn) or block
  it (because the task's frames did not install it) is not settled.

## References

- `design/03-effects.md` (the row rules and the handler barrier)
- `crates/deed-interp/src/interp.rs` (`check_row`, `RowFrame`, `Promise`)
- `crates/deed-driver/tests/rows_at_runtime.rs` (runtime row enforcement
  tests, including the row-propagation test added alongside this record)
- deed-lang/deed#575 (concurrency epic)
- deed-lang/deed#607 (this issue)
- deed-lang/deed#133 (runtime row check, `DEED6010`)
- deed-lang/deed#137 (handler barrier)
