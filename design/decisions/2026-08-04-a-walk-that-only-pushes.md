# Decision: a walk that only pushes builds one list

- Status: Accepted
- Date: 2026-08-04
- Supersedes: None
- Superseded by: None

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

Measured, in `crates/deed-driver/tests/walks.rs`, over the shipped library and the corpus,
counting only the walks whose accumulator is a list, because a walk carrying a number
allocates nothing:

```
walks that build a list and only ever push onto it    47
walks that build a list some other way                17
```

Those are today's numbers rather than the ones this shipped with. It said forty-five and
forty-four, and both were wrong: the denominator was every walk carrying an accumulator, and
the rule the measurement asked was missing the condition that lived at the call site.
`design/decisions/2026-08-05-a-walk-may-read-its-own-length.md` has the correction, and
`design/decisions/2026-08-05-a-walk-may-start-from-a-list.md` is why the first number moved
again.

The ones the rule accepts are `map`, `map_at`, `filter`, `filter_at` and everything written
like them. In each of them the accumulator appears once on each path through the body, as
`push`'s first argument or as the value a branch hands on untouched, which is what `filter`'s
`else` does. Nothing else holds one, so no intermediate list is reachable from anywhere, so
there is no reason for them to be separate lists.

## Decision

A walk whose accumulator is only ever pushed onto builds one list.

The rule is on the shape of the body, and it asks about one place: the value of a path
through it, which is what the next turn is handed.

- The value of every path is the bare accumulator or a `push` straight onto it, so the
  block that comes out of a turn is the block that went in and a turn grows it by at most
  one.
- Those are the only places the accumulator appears at all. Anywhere else is a place that
  could keep it, or a second push in a turn the condition above never sees.
- The list being walked bounds how many turns there are, so with the rule above the result
  is never longer than it, and the block can be reserved once at that size.

Both of the first two were learned rather than designed, and they are written up below
because of how.

The length is written as the walk goes, so the accumulator's length is right at every turn
and a walk that reads it gets the answer it would have got. The slack a filter leaves is
not given back, which is the same thing the rest of this backend does with everything and
is a constant factor rather than the quadratic this removes.

This is not reuse analysis and does not stand in for it. It needs no reference counts, no
layout metadata and no ownership reasoning, because it does not ask whether a value is
unshared: it arranges for there to be nothing to share.

The risk is worth writing down, because it is not the usual one. A walk that got this wrong
would not fail to compile or stop with a trap; it would hand back a different answer,
quietly, in a program that checks. So the shape test is the whole of the safety argument.

## What the first version got wrong

The rule above had two parts rather than three, and the missing one was found the way this
repository finds things: `crates/deed-driver/tests/shipped.rs` runs every test in the
shipped library through both engines and holds them to the same answer, and it said that
`intersperse` in `std/list` passed in one and not the other.

`intersperse` writes `push(push(out, sep), item)`. Every mention of `out` is a `push`'s
first argument, so the first version admitted it, and two things went wrong at once. A turn
grew the list by two while the room reserved was one a turn. And the accumulator came out of
the turn as the copy the outer `push` made, which was never given room at all, so the next
turn wrote past the end of it.

One condition rules out both, because both are the same mistake: the block that comes out
of a turn has to be the block that went in. The rule is now written that way rather than as
two separate guards, and `crates/deed-driver/tests/allocation.rs` holds the other side of
it, that a walk growing by more than one still copies.

Worth saying plainly: the shape test was wrong, it was wrong in the direction that corrupts
memory, and what caught it was a ratchet nobody wrote for this. That is the argument for
the ratchet, not for the reviewer.

## What the second version got wrong

The rule that shipped counted mentions of the accumulator and asked that each was a `push`
or a branch handed on, and separately asked that every path's value grew the list by one.
Two conditions counted over the whole body, one condition about the paths, and nothing
tying them together. A body can satisfy all of them and still be the mistake above:

```
let ahead = push(out, item)
let _ = length(ahead)
push(out, item)
```

Both mentions are pushes, so the counting is satisfied, and the value of the one path is a
push straight onto the accumulator, so the last condition is satisfied. The turn still
grows the list by two. Compiled, a walk over eight elements reserved room for eight, wrote
sixteen, and answered with a list of a length the interpreter never produced. The same
gap admitted a branch handing the accumulator on somewhere the turn's value was not, which
hands a live view of the block the walk is about to write into to whatever reads it.

The two conditions are now one question asked once. Every path's value is the bare
accumulator or one push onto it, and the number of mentions of the accumulator in the body
is the number of paths, so there is nowhere else it appears. This rejects nothing the
library or the corpus writes: the count over both is the same on either side of the change,
which is what says the tightening cost nothing.

`crates/deed-driver/tests/agreement.rs` carries the program, so the two engines have to
answer it the same, and `crates/deed-mir/tests/shape.rs` carries both shapes.

Both mistakes are the same mistake twice: a rule about a shape written as several
conditions over the body rather than as one question about the value a turn hands back.

## Drawbacks (required)

It is a rule about a shape rather than about values, so a walk one edit away from the shape
gets none of it. A reader who does not know the rule has no way to tell which of two walks
allocates quadratically. Adding `length(out)` to a body used to be such an edit, which
`design/decisions/2026-08-05-a-walk-may-read-its-own-length.md` fixed; everything else still
is.

It answers the walk and not the general case. A program that builds a list by recursion,
or that rebuilds a record in a loop, allocates exactly as it did. The walks of every other
shape are untouched.

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

## What it came to

Bytes a compiled program allocates to build a list, with the list it walks subtracted off
both sides, before and after:

```
length     written out  folded before  folded after
16         136          1224           136
64         520          17160          520
256        2056         265224         2056
1024       8200         out of memory  8200
```

A walk that builds a list now allocates the list. The 1024 row is the one to read: the
answer was always eight kilobytes and building it used to exhaust a megabyte, so what
changed is not that it got cheaper but that it runs.

What did not change is the keyed benchmark in
`design/decisions/2026-07-31-tree-vs-table-decision.md`, which still cannot reach a
thousand keys. That is correct rather than disappointing. `std/table`'s quadratic is across
calls to `set` rather than inside one walk, and the accumulator there is handed to `set`,
which could keep it. This answers the walk, and that is a different shape.

## Open Questions (required)

- Whether the rule should allow the accumulator in read-only positions such as
  `length(out)`. Answered in
  `design/decisions/2026-08-05-a-walk-may-read-its-own-length.md`: yes, and the `while`
  clause the question mentioned turned out to be a place nothing had been looking at all.

- Whether the same argument reaches a record rebuilt in a loop. Answered in
  `design/decisions/2026-08-04-a-walk-that-pushes-into-a-record.md`: it reaches the lists
  inside the record, one field at a time, and stops at the record itself, which is still
  built a turn because the walk hands it back.

- What `std/table` would take. Its cost is across calls to `set` rather than inside a
  walk, so nothing here touches it, and a keyed structure of a few hundred entries is
  still where a compiled program stops.

## References

- `design/decisions/2026-07-31-compiled-memory-reclamation.md`
- `design/hash-map-requirements.md`
- `crates/deed-driver/tests/allocation.rs`
- `crates/deed-driver/tests/walks.rs`
- deed-lang/deed#898
