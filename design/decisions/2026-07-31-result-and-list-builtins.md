# Decision: keep Result and List built in for now

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

`Result` and `List` can now be declared in user-level syntax, but that does not remove all of the language-level mechanisms that currently name them.

## Decision

Keep `Result` and `List` built in for now.

## Drawbacks (required)

Built-in types increase language surface and make these shapes special cases while adjacent types are ordinary declarations.

## Rejected Ideas (required)

- Option: move `Result` to a prelude declaration now.
  - Rejected because: this changes constructor spelling but leaves operator and checker behavior tied to `Result` semantics, adding indirection without reducing mechanisms.
- Option: move `List` to a prelude declaration now.
  - Rejected because: list literals and `for` semantics still need a language-level target shape, so the move does not remove core coupling.
- Option: replace constructor behavior with positional variants or relaxed return-only type parameter rules.
  - Rejected because: those choices address constructor ergonomics, not the broader built-in coupling that currently determines complexity.

## Open Questions (required)

- What general rule could let `?` and outcome-keyed `ensures` target a shape rather than a named built-in type.
- What non-`List` walkable shape should `for` target, if any.

## References

- `design/02-syntax.md` (open question entry on `Result` and `List`)
