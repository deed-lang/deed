# Decision: a hash is the equality walk with a different accumulator

- Status: Accepted
- Date: 2026-08-05
- Supersedes: None
- Superseded by: None

## Context

`design/hash-map-requirements.md` asks what is missing before a hash map can be
written in Deed and names three gaps. Two of them have moved since it was
written, and the movement changes which one to take next.

**Gap 1, a contiguous indexable representation, is already there.** A compiled
list is `[length][element 0][element 1]...` with every element eight bytes
(`crates/deed-codegen/src/layout.rs`), and `at` lowers to
`base + ELEMENTS + index * WORD` in `runtime::element_at`. That is O(1) indexed
access into contiguous storage, which is what the gap asked for. The page's
doubt was about evidence, and the evidence is the layout.

**Gap 2, building without pathological copying, is half closed.**
`design/decisions/2026-08-04-a-walk-that-only-pushes.md` removed the quadratic
from forty of the seventy-eight walks in the corpus. What is left is `push` at
a function boundary, where no bound is known.

**Gap 3, a hash, has a decided direction and no implementation.** The word does
not appear in the language. `crates/deed-typeck`, `crates/deed-interp` and
`crates/deed-codegen` contain no hashing of Deed values at all.

There is one more measurement, and it is the one that decides the order.
`std/table`'s `set` rebuilds the whole list through a walk when the key is
already there. **Perfect memory reuse leaves that O(n).** So the remaining
memory work makes `Table` stop exhausting memory; it does not make `Table`
fast. What makes a keyed structure fast is a hash map, and the only thing
stopping one being written today is this gap.

## Decision

A prelude function `hash`, taking one value and answering an `Int`.

```
hash(value) -> Int
```

Structural, with no trait bound, which is what deed-lang/deed#617 decided and
is the same shape structural equality already has over bare type parameters.

### The rule that makes it correct

**The hash walk is the equality walk with a different accumulator.** Everything
`crates/deed-codegen/src/equality.rs` compares, the hash absorbs, in the same
order: a list's length then its elements, an aggregate's tag then the fields of
the variant it holds.

This is the whole safety argument, and it is deliberately not "be careful".
`a == b` implies `hash(a) == hash(b)` because the two walks read the same words
in the same order; the only way to break it is to make the two walks disagree
about what a value is made of, which is a thing a reader can see rather than a
thing a reader has to remember. `walked`, `close_over` and `held_by` are the
same predicates, used from both.

Absorbing the length and the tag is not decoration. Without the length,
`[[1], [2]]` and `[[1, 2]]` hash alike; without the tag, two variants with the
same fields do. Both are cases equality distinguishes, so both are cases the
hash has to.

### One algorithm, written down

FNV-1a over 64 bits: start at `0xcbf29ce484222325`, and for each byte, exclusive-or
it in and multiply by `0x100000001b3`, wrapping. Words go in little-endian
order, which is WebAssembly's, so the two engines feed the same bytes.

The algorithm is specified rather than left to each backend, and the reason is
not tidiness. `crates/deed-driver/tests/shipped.rs` runs every test in the
shipped library through the interpreter and the compiled backend and holds them
to the same answer. A hash is an observable `Int`, so two engines computing it
differently is a program that passes in one and fails in the other, and this
repository would find out through a test failure in an unrelated module.

FNV because it is four lines and no table, which is what a workspace with no
dependencies can hold without ceremony, and because the constants are already
in this tree.

### What can be hashed

Exactly what can be compared. A value that `==` refuses, `hash` refuses, with
the same reasoning and by asking the same predicate. Closures and function
values are the ones this excludes: two closures are the same closure rather
than the same code, so their equality is identity, and an identity that is an
address is not a thing to hash.

## Drawbacks (required)

**It is not a cryptographic hash and it is not seeded.** A caller who can
choose keys can choose colliding ones, and a hash map built on this will
degrade to a linear scan under that input. This is the usual hash-flooding
exposure and it is real. There is no seed because a seed has to come from
somewhere, and in this language somewhere is a capability: `hash` is a pure
function, so it cannot be handed one. A per-run seed would also make the same
program answer differently on two runs, which `deed test` compares.

**`hash(5)` is not 5.** Every value goes through the same walk, so a small
integer key costs a multiply rather than nothing. The alternative is a rule per
type in the specification, which is the thing `layout.rs` already argues
against for widths.

**FNV is neither the fastest nor the best-distributed choice.** It is the one
that is four lines. If a measured workload shows the distribution costing more
than the machinery would, this is a decision to revisit with a number.

**This does not make `std/table` a hash map.** It makes one writable. Whether
`std` should carry one, and what happens to `Table`'s insertion order if it
does, is not decided here.

**A list's hash is O(n) every time it is asked.** Nothing is cached, because a
cache is a field on a value and values here have no room for one. A hash map
keyed by long lists pays that on every lookup.

## Rejected Ideas (required)

- Option: a trait-bounded `Hash`, so only types that implement it can be keys.
  - Rejected because: deed-lang/deed#617 decided against it and nothing here
    reopens it. Structural equality already works over bare type parameters
    with no bound, and a hash that needed one would be the first place in the
    language where a type parameter had to promise something.

- Option: let each backend hash however it likes, since only the buckets matter.
  - Rejected because: the value is an `Int` a program can print, compare and
    assert on. Two engines disagreeing about it is two engines disagreeing about
    what a program means, and `shipped.rs` exists to prevent exactly that.

- Option: hash a closure by its address or its code index.
  - Rejected because: an address is not stable across runs and a code index says
    two closures over different values are the same. Equality already refuses
    them, and refusing the same things in the same place is one rule rather than
    two.

- Option: seed the hash per run to blunt hash flooding.
  - Rejected because: the seed has to come from a capability, `hash` is pure, and
    a pure function whose answer changes per run breaks the property that a
    `test` block means the same thing twice.

- Option: SipHash, or another keyed construction.
  - Rejected because: without a seed it buys nothing over FNV against a chosen-key
    adversary, and it is substantially more code to audit in a workspace whose
    argument for having no dependencies is that what it writes is small enough to
    read.

- Option: write it in Deed, in `std`.
  - Rejected because: it would need to take a value of any shape apart, and there
    is no reflection and no trait to dispatch on. This is one of the few things
    the language genuinely cannot say about itself.

## Open Questions (required)

- Whether a hash map belongs in `std`, and what it is called next to `Table` and
  `Map`. `design/decisions/2026-07-31-tree-vs-table-decision.md` compared those
  two at a size where neither runs; the comparison is worth redoing once a third
  option exists.

- Whether the remaining copying makes a hash map built on this fast enough to be
  worth having. This is the measurement `design/hash-map-requirements.md` asked
  for and could not take, because the program that produces it could not be
  written. It can be now, and the answer should decide what the reuse work does
  next rather than the other way round.

- Whether a `Random` capability, if one ever lands, should let a *program* seed a
  map it builds while leaving `hash` itself pure.

## References

- deed-lang/deed#617, deed-lang/deed#618
- `design/hash-map-requirements.md`
- `crates/deed-codegen/src/equality.rs`
- `crates/deed-codegen/src/layout.rs`
- `crates/deed-resolve/src/resolver.rs`
