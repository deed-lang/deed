# Decision: abandoning a suspended computation

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

Every operation in a Deed handler resumes exactly once and there is no way
for the handler to say "this computation is not coming back". That is fine as
long as a suspended computation is always either resumed or discarded with the
whole program. It stops being fine the moment a scheduler holds a suspended
task, because the scheduler can finish with tasks still in the queue, and
those tasks may be sitting inside a `for` loop or holding a resource open.

OCaml's `Effect.Deep.discontinue` is the reference point: it resumes a
captured continuation by raising at the point of `perform`, so that the
ordinary `try`/`finally` unwinding runs.

The checklist from the original issue:
- a way to abandon a suspended computation
- what the abandoned computation observes
- `finally` runs when it happens
- whether a program can catch it
- the interpreter and the backend agree about it

## Decision

**`abandon` statement.** A new keyword, usable in any position a statement
can appear (including inside a handler operation body). When the interpreter
reaches it, it raises a diagnostic with code `DEED6011` and message "this
computation was abandoned by its handler". This is not a contract failure
(DEED6002, DEED6003, DEED6004) so `assert refuses` cannot catch it.
`abandon` is type-checked as diverging, so it may appear where any type is
expected.

**What the abandoned computation observes.** Because `abandon` raises
`DEED6011` at the call site, the caller's stack unwinds normally. Any
`finally` clause on an enclosing `with` block runs on the way out (interpreter
only; see Drawbacks).

**`finally` clause on `with`.** An optional `finally { ... }` clause may
follow any `with H { ... }` block. In the interpreter, the clause runs on all
exits from the `with` body: normal, `return`, contract failure, and
abandonment. In the compiled backend it is inlined after the body, so it runs
only on normal exit.

**Not catchable.** `DEED6011` is not in the set checked by
`is_contract_failure()`, so `assert refuses` does not suppress abandonment.
This is deliberate: a handler that calls `abandon` has decided the computation
is done; the caller cannot override that decision.

**Interpreter and backend agreement.** The `DEED6011` constant is declared in
both `deed-interp` and `deed-mir` and tested to be equal. The compiled backend
lowers `abandon` to `Stmt::Fail { code: DEED6011 }`, which compiles to an
`unreachable` instruction preceded by the code and message written into memory,
the same shape every other contract failure takes.

## Drawbacks (required)

The `finally` clause only runs on normal exit in the compiled backend. A trap
(from `abandon` or any contract failure) kills the process before reaching
the inlined clause. Programs that rely on `finally` for cleanup on failure will
behave differently between the interpreter and the compiled backend. This
asymmetry is documented on the `install` function in `deed-mir/src/lower.rs`.
Fixing it requires a structured exception table in the backend, which is
deferred.

## Rejected Ideas (required)

- Option: add a new `Signal::Abandon` variant distinct from `Signal::Fail`.
  - Rejected because: a separate signal variant complicates every match on
    `Signal` for no observable difference; `DEED6011` through `Signal::Fail`
    achieves the same unwind with one fewer variant.

- Option: make abandonment catchable via `assert refuses`.
  - Rejected because: the point of `abandon` is that the handler has decided
    the computation is finished. Letting the computation catch and ignore it
    undermines the mechanism.

- Option: run `finally` on all exits in the compiled backend using a landing
  pad or cleanup section.
  - Rejected because: the current backend has no exception table and adding
    one is a larger change than this issue calls for. The interpreter already
    covers the cases a scheduler would use.

## Open Questions (required)

- Whether `finally` should be allowed to use effects from the enclosing `with`
  (it currently can, since the handler is still installed).

## References

- deed-lang/deed#603 (this issue)
- deed-lang/deed#575 (parent issue)
- `crates/deed-interp/src/codes.rs` (`DEED6011`)
- `crates/deed-mir/src/lib.rs` (`codes::ABANDONED`)
- `crates/deed-mir/src/lower.rs` (`install` function)
- `crates/deed-driver/tests/dispatch.rs` (`abandon_in_an_operation_raises_deed6011`, `a_finally_clause_runs_on_normal_exit`)
- `crates/deed-interp/tests/messages.rs` (`an_abandoned_computation_says_it_was_abandoned`)
