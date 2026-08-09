# Decision: what a callee does with its argument is the fact that is missing

- Status: Accepted
- Date: 2026-08-09
- Supersedes: None
- Superseded by: None

## Context

`design/decisions/2026-07-31-compiled-memory-reclamation.md` deferred
reclamation and named the direction the measurement pointed at: a compiled
program gives nothing back, so what it allocates in total is what its memory
reached, and most of that total is copies that died the moment the next one was
made.

Two records then took most of that away without reference counting and without
reuse analysis. `2026-08-04-a-walk-that-only-pushes.md` found a property of
`for` that makes the sharing question unnecessary: a walk's accumulator is
unshared by construction, so a walk that only pushes onto it can build one list
instead of one a turn. `2026-08-05-a-walk-may-read-its-own-length.md` widened it
to the walk that numbers what it is building.

`design/hash-map-requirements.md` says in one line what is left: "What is left
is `push` at a function boundary, where no bound is known."

That sentence had no number attached. It does now, and the number is worse than
the sentence sounds. This is the same push, written twice:

```
let built = for n in source with out = [] { push(out, n) }
```

```
fn added(list: List<Int>, n: Int) -> List<Int> { push(list, n) }

let built = for n in source with out = [] { added(out, n) }
```

Bytes allocated, with the list being walked subtracted off both:

```
length     direct   through a call   ratio
8          72       360              5x
16         136      1224             9x
32         264      4488             17x
64         520      17160            33x
128        1032     67080            65x
```

The left column is the answer and nothing else. The right column is the answer
once per element, which is where every walk was before the two records above.
The ratio is `n / 2 + 1`, so the cost is not a constant per call: it is the
whole optimisation, gone, because the push moved one line.

Nothing about the callee is unusual. It takes the list, returns the list with
one more on the end, and keeps nothing. A person reading the two programs would
call them the same program. The compiler cannot, and the reason is precise:

- the caller knows its accumulator is dead after the call, because `for` says
  so, and has no way to tell the callee;
- the callee cannot see whether its argument is dead, so it copies;
- and `added` is compiled once, for every caller it has.

So the missing fact is interprocedural, and it is one fact: **does the callee
still need its argument when it returns, and does the caller?** Everything the
reuse literature calls "reuse analysis" is machinery for answering that and
acting on the answer.

## Decision

Write this down before writing any of it, and do not write any of it yet.

Three things are decided here.

**The fact to compute is named.** Not "add reference counting", not "implement
Perceus". The property is: for each parameter of each function, whether the
function's result may share storage with that parameter, and whether the
function is finished with it at every return. That is a summary per parameter,
computable bottom-up over the call graph, and it is what a caller needs in order
to know whether handing over its only reference is safe.

**The order is fixed: the fact, then the transformation.** Computing the summary
and printing it changes no program's behaviour and can be held by tests that
read it. Acting on it changes what a compiled program writes to memory. The
first is checkable against programs that already run; the second is not
checkable at all until the first is trusted.

**A wrong answer is silent data corruption, so the analysis is one-sided.**
Every function whose summary is unknown, unreachable, recursive in a way the
summary cannot see through, or reached through a value rather than a name is
answered "keeps it". That answer is always safe and sometimes slow. The opposite
default is a program that overwrites a list somebody else is still reading, and
nothing in the language or the runtime would notice: the write succeeds, the
bytes are in bounds, and a later read returns something that was never written
there on purpose. No contract catches it, because the contract holds for the
value the function returned. No test catches it reliably, because it depends on
what else happens to be alive.

That last point is why this record exists at all rather than a branch. This
language's argument is that the compiler says what it knows and refuses what it
does not. An analysis that guesses would be the first place the compiler quietly
guesses, and it would guess about the one thing nothing else in the system
double-checks.

## What has been built since

The first of the two halves, and only the first. `crates/deed-mir/src/reuse.rs`
computes the summary this record names and `deed build --reuse` prints it.
Nothing reads the answer, so no compiled program writes different bytes than it
did before this landed.

The printed word per parameter is one of four:

| | result may reach it | kept elsewhere |
| --- | --- | --- |
| `releases` | no | no |
| `returns` | yes | no |
| `retains` | no | yes |
| `keeps` | yes | yes |

Measured over the nine shipped modules: 389 boxed parameters, of which **329
are not retained** — 225 `releases` and 104 `returns`. That number is the one
worth watching, because it is what the transformation will have to work with,
and an analysis that answered `keeps` everywhere would have satisfied every
individual case above while buying nothing. It is held as a floor by
`crates/deed-driver/tests/reuse.rs`.

What answers `keeps`, and why, is unchanged from the paragraph above: a call
through a value, an operation a handler answers, anything handed to the host,
and a parameter written into a handler's `state` — the one thing in the
language that outlives the call that wrote it. Recursion converges rather than
being refused, because the summary is a least fixed point over the call graph
and two bits per parameter is a finite lattice.

Two sharpenings turned out to be free and worth having. Only a boxed type has
storage to share, so a parameter or a return type that is a number answers
without looking at the body. And the runtime helpers are a closed set this
compiler publishes, so what each of them does with its arguments is read from a
table rather than assumed: `push` hands back a fresh spine holding the
argument's elements, `join` and `upper` and concatenation write every byte they
return, `length` returns a number. That table is the one place this analysis
knows something rather than deriving it, and it knows it about its own
primitives.

## What this buys

The sentence in `design/hash-map-requirements.md` now has a table under it, and
the table says the boundary is not a rounding error. `std/list` is a module of
functions taking a list and returning a list; so is `std/table`; so is anything
anybody writes. Every one of them is on the right column.

It also makes the earlier measurement legible. `set` in a walk allocating 560,
1616, 5264 and 18704 bytes at 8, 16, 32 and 64 keys, against 144, 272, 528 and
1040 for the same walk pushing, is this table with a different function in it.
That was read as a fact about keyed structures. It is a fact about calls.

And it settles what the reuse work is for. Two documents already wanted this
machinery for different reasons; what neither of them had was the smallest
change that would show up. It is not a hash map and it is not a collector. It is
one function of two lines.

Held by `crates/deed-driver/tests/allocation.rs`, which asserts the shape rather
than the numbers: a push behind a call allocates several times the answer, and
doubling the length more than doubles that, which a constant per call would not.
A failure means something started answering the question, and this page should
be reread.

## Drawbacks (required)

Writing a record for work nobody has started is a way of appearing to have done
it. The mitigation is the test: the table above is produced by a test that runs
on every commit, so the claim decays into a failure rather than into prose.

The one-sided default means the analysis will be wrong in the cheap direction
often, and there will be no signal saying so. A function that could have reused
and did not looks exactly like one that could not. Whatever computes the summary
should be able to print it, or the first person tuning it will be guessing.

Naming the fact narrows the search. Reference counting answers a bigger question
than this and answers it at runtime, and there are programs where the bigger
answer is the right one — anything that shares a structure between two live
names, which this summary can only refuse. Deciding the small question first
means the large one gets decided later by whoever has already built the small
one, and that is not a neutral position.

## Rejected Ideas (required)

- Option: start with the transformation on a hard-coded list of prelude
  functions, since `push` is the one that matters.
  - Rejected because: `push` is not the one that matters. The measurement above
    is a user's two-line function, and `std/table`'s `set` is another. A
    hard-coded list makes the corpus faster and the language no different, and
    it would be the compiler treating its own library better than a program.

- Option: reference counting first, and reuse as a consumer of it.
  - Rejected because: the counts are the runtime cost this whole line of records
    has been avoiding, and the measurement says the common case does not need
    them. A walk's accumulator is already known unshared statically; the missing
    piece is carrying that across one call, not learning it again at runtime
    every turn. If the summary turns out to refuse too much, the counts are
    still there to add, and by then there will be a number saying how much they
    would buy.

- Option: let a function opt in, with something in the signature saying it
  consumes its argument.
  - Rejected because: it puts a representation decision in the language's
    surface, and `design/01-principles.md` is fairly clear about what that
    costs. It would also be a promise the caller has to keep rather than one the
    compiler checks, which is backwards for this language.

- Option: skip the summary and inline aggressively, so the walk can see the push
  again.
  - Rejected because: it works exactly as far as the inliner reaches, and the
    cliff moves rather than disappearing. Recursion, a function too large to
    inline, or a call through a value all put the program back on the right
    column, and the program that fell off would look identical to the one that
    did not. It is also the answer that is hardest to test, because the
    behaviour depends on a threshold.

## Open Questions (required)

- What the summary says about a closure. A call through a value has no name to
  look up, and `design/03-effects.md` already makes closures the place where
  static answers run out.
- Whether the summary is per parameter or per parameter and return position. A
  function returning a record holding its argument shares through one field and
  not the others, and the coarser answer refuses more.
- Where the summary lives. It is a fact about a function that a caller in
  another module needs, so it is either recomputed from the dependency's source
  or written into something the dependency ships, and the second one is a
  compatibility surface.
- What happens at a `test` block and at the boundary of a compiled component,
  where the caller is not a Deed program. `crates/deed-codegen/src/adapter.rs`
  now hands a component's caller a pointer into the module's memory, and a
  reuse that overwrote it would be visible outside the program.
- Whether any of this changes the deferral in
  `design/decisions/2026-07-31-compiled-memory-reclamation.md`. It should not:
  reuse takes the copies away and still gives nothing back.

## References

- `crates/deed-driver/tests/allocation.rs`, which holds the table above
- `design/decisions/2026-07-31-compiled-memory-reclamation.md`
- `design/decisions/2026-08-04-a-walk-that-only-pushes.md`
- `design/decisions/2026-08-05-a-walk-may-read-its-own-length.md`
- `design/hash-map-requirements.md`
- `design/01-principles.md`
