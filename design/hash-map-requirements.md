# Hash map requirements today

Issue [#618](https://github.com/deed-lang/deed/issues/618) asks what is missing before a hash map can be written in Deed, what each missing piece would cost, and which pieces should actually be added now.

The short answer is three gaps:

1. A contiguous indexable representation.
2. A way to build and update without pathological copying.
3. A hash.

This document lists each gap with cost and viable options, incorporates [#617](https://github.com/deed-lang/deed/issues/617), and makes a decision.

## Gap 1: contiguous indexable representation

A practical hash table needs probe-friendly storage. That means contiguous slots and frequent indexed access in tight loops.

Today, the public shape available in the language is `List`, and the relevant operations are list walks and indexed access through `at` returning `Result`. That is enough for occasional indexing but not enough evidence for hash-table-style hot indexing.

### Cost

- Language and runtime surface area increase if a new contiguous primitive is added.
- Backend work is required for layout, bounds behavior, and code generation.
- The safety story must stay aligned with existing total operations and diagnostics.

### Viable options

- Add a new contiguous built-in representation aimed at indexed access.
- Keep using `List` plus existing operations and accept that this likely does not reach hash map performance goals.
- Introduce a focused runtime primitive that is not exposed as a broad new user-facing collection abstraction.

## Gap 2: build and update without pathological copying

Hash maps are update-heavy. If every insert step copies full structure, construction and mutation-like workloads are dominated by allocation and copying.

### Cost

- Explicit mutation would add a second state model and weaken the current immutability story.
- Any new mechanism must preserve predictable semantics and the current effect discipline.
- Compiler complexity rises if we add ownership or reuse reasoning.

### Viable options

- Add explicit mutable collections and mutable operations.
- Add a dedicated builder API with a controlled mutable phase and immutable result.
- Use Perceus-style reuse analysis so purely functional updates can reuse storage in place when values are unshared.

### Evaluation

Perceus-style reuse analysis is the best fit with Deed's current design. Deed already has immutability at the language level and a single mutable runtime representation. Reuse analysis keeps source semantics immutable while capturing the runtime win where aliasing permits reuse. That avoids introducing user-visible mutation as a second model.

## Gap 3: hash

Issue [#617](https://github.com/deed-lang/deed/issues/617) closed with the decision that hashing can be structural in the same way equality is structural.

### Cost

- A structural hash definition must be specified for every type form that already participates in structural equality.
- Runtime implementation must keep hash behavior stable and consistent with equality.
- Diagnostics may be needed for any forms that are deliberately excluded.

### Viable options

- Structural hash for all hashable type forms, with no trait bound.
- Trait-bounded hashing where only some types implement hash.

### Decision from #617

Take structural hashing and do not add a trait bound. This does not reopen traits. It follows the same shape as existing structural equality over bare type parameters.

## Decision: what to add now

Additions to take now:

1. None as immediate language surface additions for hash maps.
2. Record the structural hashing decision as the hash direction when needed, with no trait bound.
3. Continue investigating reuse analysis as the preferred way to avoid copy-heavy updates, rather than adding mutation first.

Why this is the decision now:

- The repository does not yet present measurement that a hash map is needed enough to justify these additions immediately.
- The largest cost sits in representation and update mechanics, not in trait machinery.
- Structural hashing already has a viable direction from #617 with lower conceptual cost than reopening traits.

## Measurement that would change this answer

Reopen immediate additions when measured workloads show that current keyed-collection options miss target performance in realistic Deed programs. The trigger is not a microbenchmark alone. The trigger is repeated evidence from representative workloads where:

- Lookup and insert throughput is materially below required targets.
- Allocation or copying dominates runtime for keyed operations.
- The gap remains after normal algorithmic choices available in current Deed code.

If those measurements appear, first addition should be reuse-analysis-backed update behavior, then the minimum contiguous representation needed to support it.

## Update: the copying was measured, from the other side

The trigger above asked for repeated evidence that allocation or copying dominates keyed operations in realistic programs. Part of it arrived while measuring something else, and it is worth recording here because it is about this page's second gap rather than about the page that found it.

Compiled, building a list of 256 by folding onto an accumulator allocates 129 times what the answer is worth, which is a copy of the whole structure per element. `std/table` is a list its `set` copies once per key, so a keyed structure of a few hundred entries does not fit in a compiled module's memory at all: `design/decisions/2026-07-31-tree-vs-table-decision.md` found that neither `std/table` nor `std/map` survives a thousand keys.

That is one of the three conditions rather than all of them, and it is about the compiled backend rather than about a hash map, so nothing is reopened here. What it does is agree with this page from the other direction. `design/decisions/2026-07-31-compiled-memory-reclamation.md` needs reuse analysis so that a compiled program stops allocating the copies at all, and this page wants it so that keyed updates stop copying. Two documents that arrived at the same machinery for different reasons is a better argument for it than either of them makes alone.

---

## Update: two of the three gaps have moved, and the third is closed

This page was written before the compiled backend had been measured, and two of
its three gaps say something that is no longer true.

**Gap 1 is already satisfied.** It asked for "contiguous slots and frequent
indexed access in tight loops" and doubted that `List` provides it. A compiled
list is `[length][element 0][element 1]...` with every element eight bytes
(`crates/deed-codegen/src/layout.rs`), and `at` lowers to
`base + ELEMENTS + index * WORD` in `runtime::element_at`. That is O(1) indexed
access into contiguous storage. The doubt was about evidence and the evidence is
the layout; nothing needs adding.

**Gap 2 is half closed.**
`design/decisions/2026-08-04-a-walk-that-only-pushes.md` removed the quadratic
from forty of the seventy-eight walks in the corpus, without reference counting
and without reuse analysis, by finding a property of `for` that makes the
sharing question unnecessary. What is left is `push` at a function boundary,
where no bound is known.

**Gap 3 is closed.** `hash` is a prelude function, structural, with no trait
bound, as decided. See
`design/decisions/2026-08-05-a-hash-is-the-equality-walk.md`.

### Why the hash went first

The order this page implies is reuse analysis, then representation, then the
hash. That order was right when it was written and is not right now, for one
reason the page could not have known: `std/table`'s `set` rebuilds the whole
list through a walk when the key is already there, so **perfect memory reuse
leaves it O(n)**. The remaining memory work stops `Table` exhausting memory. It
does not make a keyed structure fast. What does is a hash map, and the only
thing stopping one being written was this gap.

So the measurement this page asked for and could not take — whether a keyed
structure in Deed misses its targets after the copying is accounted for — is now
takeable, because the program that produces it can be written. That answer
should decide what the reuse work does next, rather than the other way round.

---

AI assistance disclosure: This document was drafted with AI coding assistant support and then reviewed for alignment with issues #617 and #618.
