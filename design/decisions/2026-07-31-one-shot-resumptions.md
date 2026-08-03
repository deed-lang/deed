# Decision: resumptions are one-shot values

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

A handler operation today either returns a value to its call site inline or does
not return at all. Nothing captures the suspended computation. Nothing resumes
it. That is enough for pure handlers and stateful handlers, but not for
schedulers: a scheduler needs to put a suspended computation into a queue, let
other work run, and come back to it. That requires a resumption to be a value
that can be stored and called later.

`design/05-backend.md` committed to one-shot dispatch for the compiled path and
gave the reason: if an operation can resume more than once, dispatch needs real
continuations and every compiled function changes shape. If it cannot, dispatch
is a stack search and an ordinary call. That document closed with "What would
change this: a `resume` in the language, or an operation that returns to
somewhere other than where it was performed. Neither exists."

This decision asks whether that constraint is also the language's answer, or
only an implementation limit the language should be free to lift later.

## Decision

A resumption is a one-shot value.

A handler operation may receive the suspended computation as a `Resume<A, R>`
argument, where `A` is the type the caller must supply to continue the
computation and `R` is the type the enclosing `with` block eventually produces.
Calling it with an `A` resumes the computation and produces an `R`. The value
may be stored, passed to another function, or placed in a queue. It may not be
called twice.

Using a resumption twice is refused statically. The checker tracks ownership of
`Resume` values and treats each use as a move. A second use at the same or any
later site is a compile-time error, not a runtime panic. This is what `design/05-backend.md`'s
dispatch model requires and what makes schedulers, generators, and async
possible without giving each its own primitive.

## Drawbacks (required)

Introducing resumptions as a language feature is a change to
`design/03-effects.md`, to the checker, to the interpreter, and to the compiled
backend. Handlers that today return a value inline keep working; handlers that
want to suspend a computation need a new parameter and a different calling
convention for that operation.

The one-shot rule blocks backtracking and probabilistic programming. Those
require multi-shot resumptions, and the option below explains why multi-shot is
not taken now.

## Rejected Ideas (required)

- Option: nothing changes; no resumption syntax is added.
  - Rejected because: a scheduler cannot exist without it. Concurrency, if it
    ever arrives, would then need new syntax and a new machine, which contradicts
    the reason handlers were built. The value of an effect system is that one
    mechanism covers async, generators, and schedulers without each requiring
    its own primitive. Leaving resumptions out does not avoid the problem; it
    moves it to a place where the solution costs more.

- Option: resumptions are multi-shot values, callable any number of times.
  - Rejected because: `finally` can run more than once under multi-shot. Koka's
    own documentation flags this as an open problem with active research behind
    it. Any resource acquired in a `with` block, a file handle, a lock, a
    capability, can be used after it was meant to be released. The one-shot rule
    is what prevents that class of error. No static solution to the multi-shot
    `finally` problem is known yet. If one is found, this decision should be
    reopened with that solution in hand.

## Open Questions (required)

- What the row of `Resume<A, R>` is. The suspended computation still needs
  certain effects handled at the point it is resumed. The type today captures
  nothing about that requirement, which means a caller can resume a computation
  in a context where its effects are not installed and the program fails at
  runtime. Encoding the open row into the type, as `Resume<A, R uses E>`,
  is the right direction but is not yet worked out. Resolving this is required
  before resumptions ship.

  Half of it now has a shape. An effect declares row variables, its handler
  can hold values typed by one, and each call to an operation fills the
  variable in from what it was passed, so `Resume<A, R> uses r` inside a
  handler for `E<uses r>` names the row the same way the queue in `std/task`
  does. What is still open is where the *other* rows come from: a computation
  suspended inside two nested `with` blocks needs both of them back, and only
  one of them belongs to the effect that suspended it. So this is narrower
  than it was and still not answered.

  What the row variables did settle is that a scheduler does not have to wait
  for this. `std/task` ships without resumptions: a task runs to completion,
  and a task that wants to leave room for another forks the rest of itself.
  What is missing without resumptions is suspending in the middle, which is
  what generators and preemption need and what fork-and-join does not.

- Whether a `Resume<A, R>` that is never called should be an error, a warning,
  or neither. A dropped resumption silently abandons the computation it was
  meant to continue.

- Whether the static ownership check for `Resume` values belongs in the type
  checker or the effect checker, given that both passes already see every
  function and the rule is closer to a linearity constraint than an effect.

## References

- `design/05-backend.md` (Effect handlers are one-shot, and that decides the dispatch)
- `design/03-effects.md` (Handlers; Closures)
- `https://github.com/deed-lang/deed/issues/605`
- `https://github.com/deed-lang/deed/issues/575`
