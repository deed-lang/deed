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

- ~~What is the first host workload where the measured allocation limit is unacceptable in practice.~~ Answered below: a keyed structure of a few hundred entries.
- Which representation and ownership metadata are needed to stage reference counting and in-place reuse incrementally.
- ~~Whether handler frames should move to a separate reclaimed stack before full value-level reclamation lands.~~ Answered below: yes, and they have.

## Update: handler frames are reclaimed

The third open question is answered, and the answer is yes.

Handler frames now live on their own stack (`layout::FRAME_BUMP`, with the region between `FRAME_START` and `HEAP_START`) and a `with` block rewinds it on the way out. This is not the reference counting the option list rejected, and it needs none of the machinery that rejection was about: no layout metadata, no root discovery, no runtime.

What makes it safe is a rule the language already has rather than an analysis. A frame's lifetime is exactly its `with` block; `design/05-backend.md` already described the exit as the block "putting back what was there". Nothing in a program can hold a frame: the source never names one, and a frame holds the *address* of its state rather than the state. Blocks nest, so the frames in flight are a stack.

Values cannot follow, and the reason is one line: a block's value outlives the block, so rewinding the value bump pointer at the end of a `with` would free what the caller is about to read. That is the difference between the two pointers and the whole of why there are two.

Measured, in `crates/deed-driver/tests/compiled_memory.rs`:

- a walk installing one handler a turn used to allocate 48 bytes a turn and now allocates 16
- twenty thousand turns of it used to exhaust linear memory and now completes
- the frame stack is bounded, and exceeding it traps rather than writing a frame over the value heap, which is checked by a nest deep enough to do it

The sixteen bytes still leaking per turn are the handler's state cell and the unit its operation answers with. The state cell is the interesting one: `DEED4030` already refuses a closure over handler state, so its lifetime is the block as well, and it could go the same way for the same reason.

## Update: the state goes with the frame, and a `with` now costs nothing

The state cell went the same way, and the reason it could is the reason the frame could: nothing in a program can hold it. `DEED4030` refuses a closure over handler state, and an operation hands back the value in a field rather than the block holding it, so the block's lifetime is exactly the `with`. It is now reserved from the frame stack rather than the value heap, immediately under the frame that points at it, and the block rewinds past both on the way out.

What this took was not the machinery the earlier note expected. The state a handler is installed with is always written out at the `with`, so it arrives at the backend as a literal and this is a matter of choosing an allocator rather than of working out where a value came from.

Measuring it turned up something the earlier note had guessed wrong. A turn was said to leak the state cell and the unit an operation answers with. Measured against the same walk with no `with` in it, the remaining eight bytes are the walk's: a walk over numbers allocates a word a turn on its own, and nothing in it is a value that lives in memory. So the number to compare against was never zero, and with the state moved, **a walk that installs a handler every turn now allocates exactly what the same walk without one does.** Installing a handler is free.

Held by `crates/deed-driver/tests/compiled_memory.rs`, as the difference between the two walks rather than as a number, so neither the walk's own cost nor the frame's can change without the other being noticed.

What is left is values, and they cannot follow by this argument at all, because a block's value outlives the block. That is the part the measurement above is about.

## Update: the first open question is answered, and the waste has a shape

The first open question asked what the first host workload is where the limit is unacceptable in practice. It is a keyed structure of a few hundred entries, which is a log file with a few hundred distinct sources rather than anything pathological.

`design/decisions/2026-07-31-tree-vs-table-decision.md` found it while measuring something else: compiled, neither `std/table` nor `std/map` survives a thousand keys. The list reaches the end of memory copying itself, and the tree reaches it two hundred inserts later. At that size the question those two modules were compared to answer does not arise, because neither of them runs.

The shape of the waste is measured now as well, and it decides the direction. Because nothing is given back, **what a compiled program allocates in total is what its memory reached**, so the number that matters is not how much it allocates but how much of that is still worth anything. Building a list by folding `push` onto an accumulator, with the list being walked subtracted off both sides:

```
length     written out  folded       copies
16         136          1224         9x
64         520          17160        33x
256        2056         265224       129x
1024       8200         out of memory
```

The ratio is `n / 2`, which is what `1 + 2 + ... + n` over `n` comes to: the fold allocates the whole answer once per element. Every one of those copies is dead the moment the next one is made, and nothing else points at it, because the accumulator of a `for` is unshared by construction.

The last row is the whole argument in one line. **The answer is eight kilobytes and building it exhausts a megabyte.** Nothing about the result is large; what does not fit is the road to it.

That is the case reuse analysis answers, and it is most of the total. A tracing collector would reclaim the same bytes later; reuse would not allocate them at all, and would need no layout metadata, no root discovery and no runtime, which were the three reasons the collector was turned down above.

`design/hash-map-requirements.md` reached the same place from the other side. Its second gap is "build and update without pathological copying", it picks Perceus-style reuse analysis over adding mutation, and its reasoning is about this language specifically: Deed is immutable at the language level and has a single mutable runtime representation, so reuse keeps the source semantics and takes the win where aliasing permits. Two documents wanting the same machinery for different reasons is the strongest argument either of them has.

Held by `crates/deed-driver/tests/allocation.rs`, which asserts the shape rather than the number: a fold allocates on the order of a copy per turn, and building the same thing twice allocates twice. Both are counts of bytes a compiled program produces, so they are the same on every machine, and a failure means something started reclaiming or reusing and this page should be reread.

The decision itself does not change here. This is still not the change that ships a collector, and reuse analysis is still a cross-cutting compiler and runtime project rather than a surgical fix. What changed is that the deferral is now against a measured limit that has been reached and a direction the measurement points at, rather than against a limit nobody had met.

## Update: one of the programs this page was blamed for was a missing call

`examples/logs.deed` stopped compiled with "reached past the end of memory".
That is the sentence this page's measured limit produces, the program is the
one that reads a directory and builds a keyed report, and a keyed structure of
a few hundred entries is exactly what the first open question above says the
limit costs. So it was read as this page for two releases. Raising the runner's
ceiling did not change it, which was taken as confirmation.

It was `str_concat`. Eleven of the twelve runtime helpers that reserve room go
through `allocate`, which moves the bump pointer and grows the memory in the
same breath. `str_concat` moved the pointer itself and never grew, so joining
two strings once the memory was full wrote them past the end of it. The trap
came from the write rather than from the allocation, which is why raising the
ceiling changed nothing: the memory never tried to grow.

`examples/logs.deed` now runs compiled and prints the same 69,680 bytes the
interpreter prints, byte for byte.

Held two ways. `crates/deed-codegen/src/runtime.rs` asks every helper that
reserves room whether its body grows the memory, and asks a helper that
reserves nothing the same question so the two are told apart.
`crates/deed-driver/tests/compiled_memory.rs` runs a program that joins text
past the pages a module starts with; with the growth removed it comes back
`OutOfBounds`, which is the original symptom.

**What this does not change.** Everything above about reclamation stands. A
compiled program still gives nothing back, total allocation is still peak
memory, and the keyed update is still quadratic: measured at 8, 16, 32 and 64
keys, `set` in a walk allocates 560, 1616, 5264 and 18704 bytes against 144,
272, 528 and 1040 for the same walk pushing. What changed is that one program
which was being counted as evidence for that limit was not evidence for
anything.

The lesson is about the evidence rather than the machinery. A trap whose
message names a resource is not a measurement of that resource: it says a write
went somewhere it should not, and the reason it was allowed to is a separate
question. Nothing here had asked which write.

## References

- `crates/deed-codegen/src/layout.rs`
- `crates/deed-codegen/src/compile.rs`
- `crates/deed-codegen/src/run.rs`
- `crates/deed-driver/tests/compiled_memory.rs`
- `crates/deed-driver/tests/allocation.rs`
- `crates/deed-driver/examples/interpreting.rs`
- `design/hash-map-requirements.md`
- `design/decisions/2026-07-31-tree-vs-table-decision.md`
- deed-lang/deed#674
- deed-lang/deed#898
