# Decision: a walk may read its own length

- Status: Accepted
- Date: 2026-08-05
- Supersedes: None
- Superseded by: None

## Context

`design/decisions/2026-08-04-a-walk-that-only-pushes.md` builds one list for a walk whose
accumulator appears nowhere but as `push`'s first argument, and left a question open:

> Whether the rule should allow the accumulator in read-only positions such as `length(out)`.
> It keeps nothing, so it is safe, and `std/list`'s `take` reads one in its `while` clause
> rather than its body. Left out until a walk in the corpus wants it.

Two walks want it, and one of them wanted it badly enough to have been rewritten around the
refusal already. `std/hashmap`'s `range` turns a count into the list of positions below it,
which is `push(out, length(out))`. That mentions the accumulator twice, so the rule refused
it and the walk copied its whole answer every turn;
`crates/deed-driver/tests/map_memory.rs` measured seventeen of the nineteen kilobytes an
insert cost going there. The fix at the time was to carry a record of a position and a
list, which kept the list mentioned once and cost a record a turn instead.
`examples/generics.deed`'s `take` is refused for the same reason and has no such workaround:
it branches on `length(kept)` and copies.

The sentence in the open question about `take` turned out to be the more interesting half.
`std/list`'s `take` does read its accumulator's length in its `while` clause, it is compiled
in place today, and the reason is not that anything decided the read was safe. It is that
the rule read the body and a `while` clause is not in it. So the condition was a place where
a walk could hand its accumulator to anything at all and still be compiled as though nothing
could reach it. `crates/deed-driver/tests/agreement.rs` now carries the program that shows
what that was worth: a handler that keeps the first list the condition hands it answers 4
under the interpreter and 404 compiled.

## Decision

A walk may hand its accumulator, or a field of it, to `length` as often as it likes. The
`while` clause counts as part of the walk. And the rule asks what the accumulator starts
from rather than leaving that to the caller.

Reading the length is safe for two reasons together, and neither is enough alone. An `Int`
is not a way to keep a list, so the call cannot leave anything holding the block. And the
length a reserved block reports is written as the walk goes rather than set to the room
reserved, so every read answers what a walk that copied would have answered. That second
half is a property of `Helper::ListRoom` and `Helper::ListAppended` rather than of the rule,
and it was already relied on before anything said so.

`length` has to be the one the language provides. Shadowing a builtin is a warning rather
than an error, so a file can declare or import a `length` of its own, and one a program
wrote could hand the accumulator to something that keeps it. The rules in
`crates/deed-mir/src/shape.rs` read a shape and are handed the answer to that one question,
because resolving a name is not something a shape can do.

The `while` clause is read before each turn with the accumulator in scope, which is the
language's own description of it, so it is a place the accumulator appears and the count
that says "nowhere else" has to include it. Nothing in the library or the corpus is refused
by adding it: the two walks that read a length there are the two this decision now allows.

## What the counting got wrong

The rule had three conditions and only two of them lived in `shape.rs`. That the accumulator
starts from the empty list was checked by `deed-mir`'s lowering, at the call site, and
`crates/deed-driver/tests/walks.rs` asked `shape.rs` alone. So the measurement counted eight
walks that the compiler never built in one list: `concat` starts from the list it was given,
`prepend` from a one-element list, and six more like them. The number the decision record
printed was a number about a rule nothing used.

That is the same failure the third test in `walks.rs` was written for, arriving from a
direction it did not cover: it held the printed counts to what the rule said, and the rule
it asked was not the whole rule. The condition now lives with the other two, so there is one
question and one place to ask it.

The counts moved for a second reason as well. `counted()` divided every walk carrying an
accumulator into two piles, including walks that carry a number or a flag and allocate
nothing at all. The test is called `most_walks_that_build_a_list_only_ever_push_onto_it`,
and that was never the denominator it used. It is now: a walk builds a list when the checker
says its accumulator is one, which is what tells `concat` from `count`.

## Drawbacks (required)

It is still a rule about a shape. `length` is allowed and nothing else is, so a walk that
reads its accumulator through anything else, including a one-line function of its own that
calls `length`, gets none of it. The line is drawn where it is because `length` is the only
name in the language whose answer cannot hold the thing it was asked about, and working out
whether some other call could would be the analysis this whole line of work exists to avoid.

A reader still has no way to tell which of two walks allocates quadratically. This widens
the shape that does not, so there is one fewer surprising edit, and it does not change that.

The one fact the rules cannot see for themselves is now a parameter. A caller that answers
it wrongly gets a silently wrong program, which is a new way to be wrong that a rule reading
only the tree did not have. Both callers answer it from the resolver, and
`crates/deed-mir/tests/shape.rs` carries the case where the answer is no.

## Rejected Ideas (required)

- Option: allow any call whose answer is an `Int`, since an `Int` cannot hold a list.
  - Rejected because: the call can keep the list without returning it. A function that
    performs an effect can hand its argument to a handler's state, which outlives the walk,
    and that is exactly the program the `while` clause case above is made of.

- Option: allow the accumulator anywhere the checker says the value is not kept.
  - Rejected because: that is escape analysis, and the argument for this whole rule is that
    it needs none. It arranges for there to be nothing to share rather than working out
    whether sharing happened.

- Option: leave the `while` clause out and write down that it is trusted.
  - Rejected because: it was not trusted, it was unexamined, and the program in
    `agreement.rs` answers differently because of it. A rule whose safety argument is a
    count of where a name appears cannot leave out a place it appears.

- Option: keep the `starts from the empty list` condition at the call site and teach the
  measurement to ask the same question.
  - Rejected because: two copies of one rule is what the third test in `walks.rs` exists to
    catch, and this was that failure. One question in one place is the fix.

## What it came to

`std/hashmap`'s `range` is written the obvious way again and the record it carried is gone:

```
fn range(count: Int) -> List<Int> {
    let places = repeat(0, count)
    for one in places with out = [] {
        push(out, length(out))
    }
}
```

Bytes a compiled `range` allocates, before and after. The workaround had already taken the
quadratic out; what this takes out is the record it built a turn:

```
count    with the record    reading the length
16       272                272
64       2080               1040
256      8224               4112
```

The ceiling `std/hashmap` writes about in its own header moved with it. A compiled map used
to stop between two and three hundred keys and now stops between three and four hundred,
which `crates/deed-driver/tests/map_memory.rs` holds. The number is meant to move and to be
noticed moving; this is the second time.

Over the library and the corpus, of the walks whose accumulator is a list:

```
walks that build a list and only ever push onto it    39
walks that build a list some other way                21
```

## Open Questions (required)

- Whether a walk that starts from a list it was given, rather than from `[]`, can be built
  in one block too. `concat` and `prepend` are both of those, and the room to reserve is the
  length of both lists rather than of the one being walked. The list it starts from is one
  somebody else can still be holding, so the first turn would have to copy it into the
  reserved block, and nothing here has measured whether that is worth a second shape.

- Whether the record a walk hands back each turn can be written over the top of the last
  one. That is still the reuse question
  `design/decisions/2026-07-31-compiled-memory-reclamation.md` names, and removing the
  record from `range` removed the corpus's clearest example of paying for it rather than
  answering it.

- What `std/table` would take. Unchanged: its cost is across calls to `set` rather than
  inside a walk.

## References

- `design/decisions/2026-08-04-a-walk-that-only-pushes.md`
- `design/decisions/2026-08-04-a-walk-that-pushes-into-a-record.md`
- `design/decisions/2026-07-31-compiled-memory-reclamation.md`
- `design/hash-map-requirements.md`
- `crates/deed-mir/src/shape.rs`
- `crates/deed-mir/tests/shape.rs`
- `crates/deed-driver/tests/walks.rs`
- `crates/deed-driver/tests/agreement.rs`
- `crates/deed-driver/tests/map_memory.rs`
