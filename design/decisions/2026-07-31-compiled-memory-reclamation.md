# Decision: compiled backend memory reclamation is deferred behind measured limits

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

The WebAssembly backend allocates records, choices, lists, strings, closures, and handler frames in linear memory with a bump pointer. In that shape, allocation is monotonic and does not reclaim.

`crates/deed-driver/tests/compiled_memory.rs` now measures one allocating loop directly in compiled mode. The test shows two properties:

- allocation grows with turn count for a loop that builds a value each turn
- under the current fixed memory budget (16 WASM pages), the same shape eventually traps with `OutOfBounds`

So this is no longer a hypothetical risk. It is measured and reproducible.

## Decision

Do not ship a collector in this change. Do not handwave the leak either.

For now, keep the current bump-allocation backend and ratchet its behavior with executable measurements:

1. expose per-call allocation measurement (`call_measured`)
2. keep a regression test that proves loop allocation growth
3. keep a regression test that proves the finite limit is hit deterministically

This makes the current limit explicit while preserving a small backend surface.

## Drawbacks (required)

Compiled programs that allocate in long-running loops still exhaust linear memory. The decision improves observability and test coverage, not asymptotic memory behavior.

## Rejected Ideas (required)

- Option: do nothing and rely on process exit.
  - Rejected because: this backend is intended for embedding, where module memory can outlive one script-sized run.
- Option: add a tracing collector now.
  - Rejected because: it requires layout metadata, root discovery, and runtime machinery larger than this issue's minimal scope.
- Option: implement full reference counting with reuse analysis now (Perceus-style).
  - Rejected because: it is likely the right long-term direction, but it is a large cross-cutting compiler/runtime project, not a surgical fix.

## Open Questions (required)

- What is the first host workload where the measured allocation limit is unacceptable in practice.
- Which representation and ownership metadata are needed to stage reference counting and in-place reuse incrementally.
- Whether handler frames should move to a separate reclaimed stack before full value-level reclamation lands.

## References

- `crates/deed-codegen/src/layout.rs`
- `crates/deed-codegen/src/compile.rs`
- `crates/deed-codegen/src/run.rs`
- `crates/deed-driver/tests/compiled_memory.rs`
- deed-lang/deed#674
