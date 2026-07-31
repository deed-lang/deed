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

## Update: handler frames are reclaimed

The third open question is answered, and the answer is yes.

Handler frames now live on their own stack (`layout::FRAME_BUMP`, with the region between `FRAME_START` and `HEAP_START`) and a `with` block rewinds it on the way out. This is not the reference counting the option list rejected, and it needs none of the machinery that rejection was about: no layout metadata, no root discovery, no runtime.

What makes it safe is a rule the language already has rather than an analysis. A frame's lifetime is exactly its `with` block; `design/05-backend.md` already described the exit as the block "putting back what was there". Nothing in a program can hold a frame: the source never names one, and a frame holds the *address* of its state rather than the state. Blocks nest, so the frames in flight are a stack.

Values cannot follow, and the reason is one line: a block's value outlives the block, so rewinding the value bump pointer at the end of a `with` would free what the caller is about to read. That is the difference between the two pointers and the whole of why there are two.

Measured, in `crates/deed-driver/tests/compiled_memory.rs`:

- a walk installing one handler a turn used to allocate 48 bytes a turn and now allocates 16
- twenty thousand turns of it used to exhaust linear memory and now completes
- the frame stack is bounded, and exceeding it traps rather than writing a frame over the value heap, which is checked by a nest deep enough to do it

The sixteen bytes still leaking per turn are the handler's state cell and the unit its operation answers with. The state cell is the interesting one: `DEED4030` already refuses a closure over handler state, so its lifetime is the block as well, and it could go the same way for the same reason. That is the next step and it is deliberately not taken here, because it needs the state's address to stop being a heap address and every operation call reads it.

## References

- `crates/deed-codegen/src/layout.rs`
- `crates/deed-codegen/src/compile.rs`
- `crates/deed-codegen/src/run.rs`
- `crates/deed-driver/tests/compiled_memory.rs`
- deed-lang/deed#674
