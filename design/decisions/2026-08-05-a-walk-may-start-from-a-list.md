# Decision: a walk may start from a list

- Status: Accepted
- Date: 2026-08-05
- Supersedes: None
- Superseded by: None

## Context

`design/decisions/2026-08-04-a-walk-that-only-pushes.md` builds one list for a walk whose
accumulator is only ever pushed onto, and it asked one thing of what the walk starts from:
that it be `[]`. That condition was there because a reserved block starts empty, and it left
out `concat` and `prepend`, which start from a list they were handed.

`design/decisions/2026-08-05-a-walk-may-read-its-own-length.md` measured what that cost. Of
the walks in the library and the corpus whose accumulator is a list, eight had the shape and
were refused for what they started from, `std/list`'s `concat` and `prepend` among them.
That record left the question open and said what the answer would have to deal with: the
list a walk starts from is one somebody else can still be holding.

## Decision

A walk may start from any list. The block is reserved as long as what it started from plus
the list it walks, the first thing the walk does is copy what it started from into that
block, and after that it is the walk that was already answered.

The copy is what makes it safe, and it is one copy rather than one a turn. Whoever handed
the list over still holds it, so appending to it where it stands would change a value under
somebody who never asked. Copying it into a block nothing else can reach puts the walk back
in the position the empty case was already in: the only list anything can see is the one the
next turn is handed.

The rule in `crates/deed-mir/src/shape.rs` no longer asks what the accumulator starts from.
It asks about the body and the `while` clause, which is what it was always about, and a walk
that only pushes carries a list whatever it started from. What is left is a question about
the type rather than the shape, and the lowering is where the type is known, so that is
where it is asked. The measurement asks the checker the same question, so the two cannot
drift: `crates/deed-driver/tests/walks.rs` counts walks whose accumulator the checker says
is a list.

## Drawbacks (required)

The copy is work the empty case does not do. It is linear in what the walk started from and
happens once, against a fold that copied the whole accumulator every turn, so it is not
close; but a walk over an empty list that starts from a long one now copies the long one for
nothing, where before it handed it straight back.

The slack is still never given back. A walk that starts from a thousand and adds two
reserves a thousand and two and holds them for as long as the answer lives.

## Rejected Ideas (required)

- Option: append to the list the walk started from, where it stands.
  - Rejected because: it is somebody else's list. That is the uniqueness question this whole
    line of work exists not to ask, and getting it wrong changes a value under a caller that
    never asked for it, quietly.

- Option: reserve only what the walk adds and keep the two lists apart, joining them at the
  end.
  - Rejected because: the accumulator has to be one list on every turn, since the body reads
    it as one and hands it on as one. Two lists behind one name is a representation change
    for every list in the language.

- Option: leave it, since `concat` is not the common shape.
  - Rejected because: it is eight of the thirteen walks that were left, and the two in
    `std/list` are the ones every program that joins two lists goes through.

## What it came to

Bytes a compiled walk allocates joining two lists of `n`, before and after:

```
n      before          after
8      1152            424
32     13728           1576
128    202272          6184
512    out of memory   24616
```

Quadratic to linear, and the row that used to run out of memory now runs.

Over the library and the corpus, of the walks whose accumulator is a list:

```
walks that build a list and only ever push onto it    47
walks that build a list some other way                13
```

`crates/deed-driver/tests/agreement.rs` carries the program that says the copy is a copy: a
walk starting from a list the caller reads afterwards, so a block appended to where it stood
would answer differently.

## Open Questions (required)

- Whether the record a walk hands back each turn can be written over the top of the last
  one. Unchanged, and still the reuse question
  `design/decisions/2026-07-31-compiled-memory-reclamation.md` names.

- Whether a walk over an empty list should skip the copy. It is a branch on a length that is
  already loaded, and nothing has measured wanting it.

## References

- `design/decisions/2026-08-04-a-walk-that-only-pushes.md`
- `design/decisions/2026-08-05-a-walk-may-read-its-own-length.md`
- `crates/deed-mir/src/shape.rs`
- `crates/deed-codegen/src/runtime.rs`
- `crates/deed-driver/tests/walks.rs`
