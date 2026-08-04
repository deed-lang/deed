# Decision: a walk that pushes into a record builds one list per field

- Status: Accepted
- Date: 2026-08-04
- Supersedes: None
- Superseded by: None

## Context

`design/decisions/2026-08-04-a-walk-that-only-pushes.md` decided that a walk whose
accumulator is only ever pushed onto builds one list rather than one a turn, and measured
what that was worth: building a list of 256 went from 129 times the answer to exactly the
answer, and a list of 1024 went from exhausting the module's memory to running.

It left this in its open questions, as the next thing to measure: whether the same argument
reaches a record rebuilt in a loop. It does, and the reason it had to be asked separately
is that the shape looks nothing like the one that rule is about. `std/list` writes:

```
for item in items with parts = Parts { kept: [], rest: [] } {
    if keep(item) {
        Parts { kept: push(parts.kept, item), rest: parts.rest }
    } else {
        Parts { kept: parts.kept, rest: push(parts.rest, item) }
    }
}
```

No mention of the accumulator is a `push`'s first argument, so the earlier rule refuses it
and every turn copies both lists. Measured, with the list being walked subtracted off, a
walk of this shape over 256 elements allocated 267304 bytes for an answer worth about two
thousand, and 1024 did not run.

The four walks in the shipped library that carry a record are `partition`, `unzip`, `std/hashmap`'s `range` and
`scan`, which is to say both sides of a filter, the inverse of `zip`, and every partial
fold. None of them could be used on a keyed structure of a few hundred entries.

Measured, in `crates/deed-driver/tests/walks.rs`:

```
walks that carry a record with a field built in place    4
fields those walks build in place                        6
```

## Decision

A field of a record accumulator that is only ever pushed onto is one list for the whole
walk.

The rule is the earlier one asked once per field, and it rests on the record being
unreachable for exactly the reason the list was. Three things have to hold:

- The field starts as an empty list, because a reserved block starts empty.
- The value of every path through the body is a record literal whose entry for that field
  is either that field read off the accumulator or one `push` onto it, so the block that
  comes out of a turn is the block that went in.
- Those are the only places the field is read, and every mention of the accumulator itself
  is a field read, so nothing holds the record and nothing holds the field through it.

A field that fails any of them is built a turn as it always was, which is what `scan` needs:
its accumulator is a `Pair` whose left is an ordinary value handed to the step function and
whose right is the list being built.

The record itself is still built a turn. It is a fixed size, so that is linear in the
length of the walk rather than quadratic, and writing the next one over the top of the last
would be a claim about whether anything holds it, which is the question this whole line of
work exists to avoid asking. It is also the smaller half of what is left.

## Drawbacks (required)

The record is still built a turn, so a walk of this shape allocates about four times what
its answer is worth at any length rather than about one. That is a constant, and the earlier
rule left a constant too, but the number is bigger and it is the obvious thing a reader
would expect to be gone.

It is a rule about a shape, so it splits hairs a reader cannot see. Reading `length` of a
field somewhere in the body takes that field off the fast path and leaves the other one on
it, and nothing says so.

It only reaches one level. A record holding a record holding a list is refused, and so is a
list of lists where the inner ones are built by the same walk.

Two rules now say nearly the same thing about two shapes. A third shape would want a third,
and at some point the answer is the general one rather than another special case.

## Rejected Ideas (required)

- Option: write the record over the top of the last one as well, so a turn allocates
  nothing at all.
  - Rejected because: the record is what the walk hands back, so the last one written is
    the answer and is read after the walk ends. Deciding that no earlier one is read is the
    same reachability question reuse analysis answers properly, and the whole point of both
    of these rules is that they arrange for there to be nothing to share instead of proving
    that nothing does. The cost of leaving it is linear and small.

- Option: generalise both rules into one that follows lists through any structure.
  - Rejected because: following a list through a structure is asking where a value can get
    to, which is the analysis neither of these rules needs. Two shapes are what the library
    writes; a rule for shapes nobody has written would be guessing at which ones matter.

- Option: leave the record accumulator alone, since only three library functions carry one.
  - Rejected because: the three are `partition`, `unzip` and `scan`, and what the count
    measures is how many ways the library found to need it rather than how much code is
    affected. A library where the filter that keeps both sides cannot be used past a few
    hundred elements has a hole in it wherever that function is the right answer.

## What it came to

Bytes a compiled program allocates for `partition` over a list, with the list it walks
subtracted off, before and after:

```
length     before         after
16         1384           544
64         17704          2080
256        267304         8224
1024       out of memory  32800
```

Linear where it was quadratic, and the 1024 row runs. What is left is thirty-two bytes a
turn: the two words of the record built each turn, and one slot in each of the two reserved
blocks.

## Open Questions (required)

- Whether the record itself can be written over. It is the one allocation a turn that is
  left here, and it is the same question reuse analysis answers, so this is where the two
  lines meet rather than a fourth special case.

- Whether the two rules should be one. They already share the argument and most of the code
  that asks it; what they do not share is a way of saying "this position keeps nothing"
  that would cover both and the next shape too.

- What `std/table` would take. Unchanged from the earlier record: its cost is across calls
  to `set` rather than inside a walk, and a keyed structure of a few hundred entries is
  still where a compiled program stops.

## References

- `design/decisions/2026-08-04-a-walk-that-only-pushes.md`
- `design/decisions/2026-07-31-compiled-memory-reclamation.md`
- `design/hash-map-requirements.md`
- `crates/deed-driver/tests/allocation.rs`
- `crates/deed-driver/tests/walks.rs`
- `crates/deed-mir/tests/shape.rs`
