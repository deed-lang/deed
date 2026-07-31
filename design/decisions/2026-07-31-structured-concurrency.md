# Decision: tasks use structured concurrency and cannot outlive the block that started them

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

Once tasks exist, the question is what happens when the block that started a task finishes
first. Two shapes are possible. The first is detached spawn: the task keeps running after
its creator exits, with no owner and no scoped lifetime. The second is structured
concurrency: the task is tied to the block that started it, so the block does not exit
until every task it started has finished.

The question matters now because the choice is not freely reversible. A language that
starts with detached spawn and then adds structured concurrency has both, with all the
complexity that comes from two overlapping ownership models. A language that starts with
structured concurrency and later adds detached spawn can do so deliberately with full
knowledge of what it is giving up.

The current handler model provides an analogy. `with H { ... }` installs a handler for
the lifetime of its block and removes it when the block exits. The effect is handled
inside and unhandled outside, so the lifetime is explicit and the scope is named. A task
group would say exactly the same thing: inside this block, these tasks run; outside it,
they do not exist. The mechanism is the same, and a task block falls out of the same
`finally`-style cleanup the handler model already relies on.

## Decision

Tasks cannot outlive the block that started them. Detached spawn is refused. When
concurrency arrives, tasks will be started and joined inside a `with`-shaped block, and
the block will not return until all of them have finished.

The refusal is enforced now rather than when tasks arrive. `spawn(f())` at statement level
is refused by the parser with `DEED2014`, which gives a message that explains the
structured-concurrency decision rather than treating `spawn` as an unknown name.

## Drawbacks (required)

Structured concurrency prevents fire-and-forget tasks. A program that wants to start a
background job and not wait for it has no direct mechanism, and would need to either
redesign so the waiting happens at a natural block boundary, or accept that the task is
part of the block's lifetime.

Composition is harder at the boundary where two task groups need to share a resource. Both
groups are scoped, but joining one before starting the other is sequential, not concurrent.
The shape for sharing ownership between concurrent scoped blocks is not yet settled.

## Rejected Ideas (required)

- Option: allow detached spawn as the primary concurrency primitive.
  - Rejected because: every language that started with detached spawn later added
    structured concurrency and then carried both forever, paying the complexity of two
    ownership models. Deed has refused several things that other languages carry for
    compatibility and regret; detached spawn is the same shape of decision.

- Option: allow detached spawn alongside structured concurrency.
  - Rejected because: the goal of refusing is to have one ownership model, not two. If
    detached spawn is available, the structured block buys nothing that a disciplined use
    of detached spawn would not provide, and the structured block stops being a guarantee
    and becomes a convention.

- Option: decide later, when tasks are implemented.
  - Rejected because: the choice is cheaper to make before any spawn syntax is in use.
    Once `spawn(f())` means something in the language, removing it costs migration. Making
    the refusal explicit now keeps the option open and makes the reasoning visible before
    any program relies on the other reading.

## Open Questions (required)

- What the `with`-shaped syntax for a task group looks like, and how results are
  collected from tasks that return values.
- What happens when one task in a group fails: whether the group cancels the rest or
  waits for all of them to finish or propagate.
- What happens when the block is abandoned from outside, for example by a `return` from
  the containing function or by a `?` propagating an error through it.
- Whether a task group needs an explicit effect row entry, given that running tasks in a
  block is itself a capability that changes what the block can do.

## References

- `design/03-effects.md` (the `with` block and handler lifetime)
- `conformance/cases/reject-spawn/` (the test where a parent would return before its
  child, which is refused)
- `DEED2014` in `crates/deed-parser/src/codes.rs`
- `https://github.com/deed-lang/deed/issues/609`
