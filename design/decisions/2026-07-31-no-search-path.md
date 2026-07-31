# Decision: module imports have no search path

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

A named module needs one place to resolve from. Search paths, manifests, and config files add more than one place and can disagree.

## Decision

Deed resolves modules from the root implied by the file named on the command line. There is no search path, no config file, and no manifest.

## Drawbacks (required)

A project cannot keep dependencies in separate roots and ask the compiler to search across them. The layout is strict and can feel rigid for existing multi-root setups.

## Rejected Ideas (required)

- Option: support a configurable search path.
  - Rejected because: the same `use` line could resolve differently across machines, and import resolution stops being visible in source alone.
- Option: add a manifest that lists module roots.
  - Rejected because: it duplicates information that file paths already provide and introduces a second artifact that can drift.

## Open Questions (required)

- Whether future package management should add a separate distribution layer without changing local source resolution.

## References

- `design/02-syntax.md` (Modules, and "A library that ships with the compiler")
