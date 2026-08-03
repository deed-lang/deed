# Decision: a walk that only pushes should build one list

- Status: Proposed
- Date: 2026-08-04
- Supersedes: None
- Superseded by: None

**Nothing here is written yet.** The measurement is, and it is what this page is for: the
shape below is the majority of the walks in the library and the corpus, which is the thing
that had to be true before any of the machinery was worth writing. What is decided is which
machinery, and the code is the next change rather than this one.

## Context

`design/decisions/2026-07-31-compiled-memory-reclamation.md` measured what a compiled
program does with memory and found the shape of the waste. Because nothing is given back
except a handler frame, what a program allocates in total is what its memory reached, and
building a list of 256 by folding `push` onto an accumulator allocates 129 times what the
answer is worth. At 1024 the answer is eight kilobytes and building it exhausts the
module's megabyte.

That page names reference counting with reuse analysis as the long-term direction and turns
down a tracing collector, and `design/hash-map-requirements.md` picked the same machinery
from the other side. Neither is a small change. The question this decides is whether there
is something smaller that answers most of the measured waste without any of it.

There is, and the reason is a property this language has and most do not. `for` is the only
loop, so every list a program builds is built by one, and a `for` is a fold: the
accumulator is bound again each turn rather than assigned. So the intermediate lists exist
only as values of the accumulator, and whether any of them can be observed is a question
about what the body does with that one name.

Measured, in `crates/deed-driver/tests/walks.rs`, over the shipped library and the corpus:

```
walks whose accumulator is only ever pushed onto     44
walks of every other shape                           34
```

The 44 are `map`, `map_at`, `filter`, `filter_at` and everything written like them. In each
of them the accumulator appears only as `push`'s first argument, or as the value of a
branch handing it on untouched, which is what `filter`'s `else` does. Nothing else holds
one, so no intermediate list is reachable from anywhere, so there is no reason for them to
be separate lists.

## Decision

A walk whose accumulator is only ever pushed onto should build one list.

The rule is on the shape of the body, and both halves of it are load-bearing:

- Every mention of the accumulator is the first argument of a `push`, or the value of a
  branch, and nothing else. A mention anywhere else is a place that could keep it.
- The list being walked bounds how many turns there are, so the result is never longer than
  it, and the block can be reserved once at that size.

The length is written as the walk goes, so the accumulator's length is right at every turn
and a walk that reads it gets the answer it would have got. The slack a filter leaves is
not given back, which is the same thing the rest of this backend does with everything and
is a constant factor rather than the quadratic this removes.

This is not reuse analysis and does not stand in for it. It needs no reference counts, no
layout metadata and no ownership reasoning, because it does not ask whether a value is
unshared: it arranges for there to be nothing to share.

The risk is worth writing down before anybody starts, because it is not the usual one. A
walk that got this wrong would not fail to compile or stop with a trap; it would hand back
a different answer, quietly, in a program that checks. So the shape test is the whole of
the safety argument and belongs where it can be read, and the change wants breaking on
purpose in both directions before it lands: a body that mentions the accumulator somewhere
else must not take this path, and a walk that takes it must answer what it answered before.

## Drawbacks (required)

It is a rule about a shape rather than about values, so a walk one edit away from the shape
gets none of it. Adding `length(out)` to a body would be such an edit under the rule as
written, even though reading a length keeps nothing, and a reader who does not know the
rule has no way to tell which of two walks allocates quadratically.

It answers the walk and not the general case. A program that builds a list by recursion,
or that rebuilds a record in a loop, allocates exactly as it did. The 34 walks of every
other shape are untouched.

The slack is never given back. A filter that keeps one element in a thousand reserves the
thousand and holds it for as long as the answer lives.

## Rejected Ideas (required)

- Option: reference counting with in-place reuse, the direction both pages name.
  - Rejected because: it is still the right long-term answer and still a cross-cutting
    compiler and runtime project. Nothing here makes it harder or unnecessary. This takes
    the measured majority of the waste out of the way first, so that when reuse arrives it
    is measured against a backend that is not obviously wasteful.

- Option: a capacity field on every list, so `push` can extend in place when there is room.
  - Rejected because: extending in place is only safe when the list is unshared, which is
    the uniqueness question this avoids. It also changes the representation of every list
    in the language for the benefit of one shape.

- Option: recognise `push` in a fold and grow geometrically.
  - Rejected because: the walk already knows exactly how long the answer can be, since the
    list it walks bounds it. Doubling would allocate more and answer a question nobody
    asked.

- Option: leave it, on the grounds that the interpreter is fine and the backend is young.
  - Rejected because: the backend is what a host embeds, and a keyed structure of a few
    hundred entries is not a workload anybody would call demanding.

## Open Questions (required)

- Whether the rule should allow the accumulator in read-only positions such as
  `length(out)`. It keeps nothing, so it is safe, and `std/list`'s `take` reads one in its
  `while` clause rather than its body. Left out until a walk in the corpus wants it.

- Whether the same argument reaches a record rebuilt in a loop. The accumulator of
  `for ... with cell = Cell { .. }` is unreachable by the same reasoning, and a record's
  size does not change, so the block could be written over rather than reserved again.
  That is nearer to reuse analysis than this is, and it is the next thing to measure.

- What is left after this, measured rather than assumed. The 129 becomes something and
  this page should say what.

## References

- `design/decisions/2026-07-31-compiled-memory-reclamation.md`
- `design/hash-map-requirements.md`
- `crates/deed-driver/tests/allocation.rs`
- `crates/deed-driver/tests/walks.rs`
- deed-lang/deed#898
