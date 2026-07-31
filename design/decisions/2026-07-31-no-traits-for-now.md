# Decision: no traits for now

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

The question is not whether traits are familiar. The question is whether they are required for this language to express its current goals.

## Decision

Do not add traits now.

## Drawbacks (required)

Passing comparator and behavior functions is more verbose than trait-based dispatch. Consistency constraints between related operations are harder to enforce statically.

## Rejected Ideas (required)

- Option: add traits now for generic ordering and printing.
  - Rejected because: measured examples remained writable with passed functions, so the implementation and complexity cost is not yet justified.
- Option: add traits now to solve constructor ergonomics around `ok` and `err`.
  - Rejected because: constructor placement is a separate problem from trait dispatch and has smaller targeted options.

## Open Questions (required)

- Whether a larger corpus can show programs that are unwritable, not only less ergonomic, without traits.
- How to express and enforce comparator consistency across operations on one keyed structure.

## References

- `design/02-syntax.md` (open question entry on traits)
- `https://github.com/deed-lang/deed/pull/246`
