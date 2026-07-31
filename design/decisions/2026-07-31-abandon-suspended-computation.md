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
ordinary cleanup unwinding runs.

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
`DEED6011` at the call site, the caller's stack unwinds normally. The existing
`finally` block on each installed handler runs on the way out while that
handler and its state are still available.

**Cleanup effects belong to the installer.** A handler's `finally` block has
no signature of its own. Its inferred effects are included in the handler row,
so installing a handler whose cleanup writes still requires `uses Io.write`.
The handler's own effect is discharged by the surrounding `with`, exactly as
it is for operation bodies.

**Not catchable.** `DEED6011` is not in the set checked by
`is_contract_failure()`, so `assert refuses` does not suppress abandonment.
This is deliberate: a handler that calls `abandon` has decided the computation
is done; the caller cannot override that decision.

**Interpreter and backend representation.** The `DEED6011` constant is
declared in both `deed-interp` and `deed-mir` and tested to be equal. The
compiled backend lowers `abandon` to `Stmt::Fail { code: DEED6011 }`, which
compiles to an `unreachable` instruction preceded by the code and message
written into memory, the same shape every other runtime failure takes.

## Drawbacks (required)

The compiled backend represents abandonment as a trap and does not yet lower
handler finalizers into an unwind path. Programs that rely on handler cleanup
after a compiled failure therefore still differ from the interpreter. Fixing
that requires structured unwind support in the backend rather than another
surface syntax, and is deferred with the existing compiled-handler limitation.

## Rejected Ideas (required)

- Option: add a new `Signal::Abandon` variant distinct from `Signal::Fail`.
  - Rejected because: a separate signal variant complicates every match on
    `Signal` for no observable difference; `DEED6011` through `Signal::Fail`
    achieves the same unwind with one fewer variant.

- Option: make abandonment catchable via `assert refuses`.
  - Rejected because: the point of `abandon` is that the handler has decided
    the computation is finished. Letting the computation catch and ignore it
    undermines the mechanism.

- Option: add `finally` after each `with` expression.
  - Rejected because: cleanup belongs to the handler that owns the resource,
    and handlers already have one `finally` block. A second cleanup syntax
    would give the same installation two competing owners.

- Option: run handler finalizers on compiled traps using a landing pad.
  - Rejected because: the current backend has no structured unwind mechanism,
    and adding one is larger than deciding and representing abandonment.

## Open Questions (required)

- What structured unwind representation should carry handler finalizers in the
  compiled backend.

## References

- deed-lang/deed#603 (this issue)
- deed-lang/deed#575 (parent issue)
- `crates/deed-interp/src/codes.rs` (`DEED6011`)
- `crates/deed-mir/src/lib.rs` (`codes::ABANDONED`)
- `crates/deed-mir/src/lower.rs` (`Stmt::Abandon` lowering)
- `crates/deed-driver/tests/dispatch.rs` (`abandon_in_an_operation_raises_deed6011`, `abandonment_runs_handler_cleanup`)
- `crates/deed-interp/tests/messages.rs` (`an_abandoned_computation_says_it_was_abandoned`)
