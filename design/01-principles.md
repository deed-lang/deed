# Principles

These are constraints, not preferences. Each one can be checked against a proposal, and
each one rules real things out. If a principle never rejects anything, it is decoration
and should be deleted.

Every principle here traces back to one of the four metrics in
[00-motivation.md](00-motivation.md).

---

## P1. Context radius of one

*Writing or verifying a function body requires reading nothing but its own signature.*

**Rejects:** global mutable state, implicit conversions, inheritance, method resolution
that depends on runtime type, exceptions crossing frames, ambient IO, decorators that
rewrite the thing they wrap, dependency injection containers, aspect-oriented anything.

**Test:** take any function in isolation, with only the signatures of what it calls. If you
cannot tell whether it is correct, the language broke this principle somewhere.

This is the principle everything else serves. It is also the one most likely to be found
wrong in practice, so it should be attacked first.

---

## P2. The specification fits in one context window

*A reader should be able to hold the entire language, including its standard library
conventions, without prior familiarity.*

**Target:** under 20 pages for the language, under 40 including the core library.

**Rejects:** feature accretion, syntactic sugar with no semantic weight, multiple
mechanisms for the same job, a standard library with three ways to build a string.

There is no corpus of Deed code and there will not be one for a long time. A language nobody
can be expected to have memorized has to be readable in full instead. This makes spec size
a first-class budget, tracked like binary size, and it is the strongest force in the whole
design against adding things.

Corollary: syntax should look familiar on purpose. Rust and TypeScript shapes are used
wherever there is no reason to differ. Novel syntax buys nothing and costs recognition.

---

## P3. Checked redundancy is good, unchecked redundancy is a liability

*Restating information is worthwhile exactly when a machine verifies both statements agree.*

**Accepts:** type annotations, preconditions, postconditions, effect rows, exhaustiveness.

**Rejects:** doc comments that describe behaviour, naming conventions that encode types,
anything asking a reader to trust that two places stayed in sync.

Redundancy is error detection. Unverified redundancy is error detection that lies to you,
which is worse than none.

---

## P4. One canonical form

*There is one way to express a given thing, and formatting is not configurable.*

**Rejects:** optional syntax, alternate spellings, style options, operator overloading,
user-defined precedence, macros.

Ambiguity costs more than verbosity. Consistent form means diffs carry signal, review gets
cheaper the more code you have seen, and generated output stops varying for no reason.

`deed fmt` is what this means in practice. It has no options for the output, not "none yet",
and there is a test asserting that every `.deed` file in the repository is already in
canonical form. Until that test existed, P4 described how the files happened to have been
typed.

Two decisions in there are worth arguing with. Parentheses are reconstructed from
precedence, because the tree does not record where they were written, so `(a - b) - c`
comes back as `a - b - c` and the source loses a hint the author may have meant. And
manual alignment goes: `a.units   == b.units` becomes `a.units == b.units`, which is the
trade P4 asks for made visible in a file someone will be annoyed about.

---

## P5. Nothing implicit crosses a boundary

*Every effect, every allocation strategy, every failure mode is declared at the boundary it
crosses.*

**Rejects:** ambient authority, implicit IO, exceptions, hidden control flow, implicit
`await` colouring, silent numeric coercion.

If a signature does not mention it, the body cannot do it. This is what makes absence
meaningful.

---

## P6. Illegal states cannot be written down

*Prefer making a wrong program unrepresentable over detecting it.*

The most effective way to raise first-attempt correctness is to shrink the space of
programs that can be expressed at all. Sum types over sentinel values, refinement types
over runtime validation, typestate over documented ordering rules.

---

## P7. Diagnostics are an API

*Compiler output is structured, ordered by likely cause, one failure at a time, and carries
an applicable patch where the fix is unambiguous.*

**Rejects:** cascading errors from a single root cause, unstable diagnostic ordering, error
text as the only machine-readable surface.

Every diagnostic has a stable code, a machine-readable form, and a human-readable rendering
built from the same data. The human rendering is a view, not the source of truth.

A diagnostic is filed against one file, which is where it gets reported, and every label on it
says which file it is about. Those are two different questions and for a while there was one
answer to both: a label carried a span and no file, so a producer holding a span from another
module had to choose between drawing a caret over whatever sat at those byte offsets in the
wrong file and saying nothing at all. They all chose to say nothing, correctly, and a reader
lost the half of the failure that was not local. A precondition failure is filed against the
caller because that is whose bug it is, and the clause it broke is in the callee; a broken
promise is filed against the function, and the call that caught it can be anywhere.

`deed fix` is what this means in practice, the same way `deed fmt` is for P4. It applies every
fix marked machine-applicable, re-checks, and repeats until nothing changes. It never
applies a fix marked maybe-incorrect, and there is no flag to make it, because a flag for
applying guesses is a flag someone turns on once and then forgets about.

Two of its rules are about declining. Fixes whose spans overlap are both dropped, since
applying either would leave the other pointing at text that moved and no order makes both
right. A fix is refused whole rather than in pieces, because a repair that wraps something is
two edits and half of it is not a smaller repair. And a fix that leaves more errors than it
found is treated as a compiler bug and nothing is written, which is the version of the check
that catches a fix that was wrong rather than merely unhelpful.

A fix is usually a span and a replacement, so most of them are written where the problem is
found. The row diagnostics are the exception and were missing for a while because of it:
`DEED5001` names the effect, names the function and tells the reader to add it to the `uses`
clause, and saying that as a span means knowing about commas, about indentation, and about a
clause that may not exist yet. The effect checker has the answer and no business knowing any
of that, so the driver writes those, where the text, the tree and the one canonical layout are
all in scope. It declines when the contract holds a `where` or an `ensures`, because nothing in
the tree says where one clause stops and the next starts, and when a comment sits in the
region, because a machine-applicable fix that deletes a comment is a fix nobody should have
applied.

---

## P8. The default is deterministic

*Time, randomness, IO, scheduling and iteration order are effects. Code that declares none
of them produces the same result every time.*

Flaky tests are wasted attempts, and attempts are the multiplier in the cost model. Also,
determinism makes replay possible, which is most of what durable execution needs anyway.

---

## P9. Compilation is fast enough to sit inside the loop

*Check latency is a language feature and it is budgeted.*

**Target:** incremental check under 100ms for a single-function edit, on a realistic
codebase.

**Rejects:** whole-program type inference, macro expansion at check time, anything that
makes editing one function require re-checking things that did not change.

### What is measured, and what is not

`deed check --timings` reports wall time per pass. On one developer machine, an unoptimised
build checking the twenty-seven files in `examples/` takes about 24ms. That is a number from one
machine and a debug build, not a guarantee, and what it grows like matters more than what it
is: `crates/deed-driver/tests/scaling.rs` builds modules of increasing size and fails if
checking stops being close to linear in the number of functions.

They are also not the target. The target is about the edit loop, and the edit loop is what
the language server does: recheck the workspace, wait for a keystroke, recheck it again.
`cargo run -p deed-driver --example edit_loop --release` measures that, and on one machine it
says:

```text
files    cold      recheck   per file   unchanged
1        0.1ms     0.1ms     56us       0%
8        0.7ms     0.6ms     69us       87%
32       2.1ms     2.1ms     67us       97%
128      8.2ms     9.2ms     72us       99%
512      37.8ms    35.8ms    70us       100%
```

Three things fall out of it, and they are worth having written down.

**It is linear, and the constant is about 70 microseconds per file.** Flat across two orders
of magnitude, which says there is no accidental quadratic behaviour hiding in the boundary
between modules, and that is the failure this measurement was most likely to find.

**The target holds today and the shape says when it stops.** At 512 files a keystroke costs
about 38ms, inside the 100ms budget. A few thousand files is where it leaves. Nobody has
written a few thousand files of Deed and the honest reading is that this is fine now.

**Essentially all of the work is repeated.** Past a handful of files, 99% of a recheck is
spent on files that did not change. So a cache would take almost all of it off, and that is
the number that says a cache is worth writing when the size arrives, rather than a feeling
that it might be.

What is deliberately not done is the cache. A cache justified by a number is a different
thing from a cache justified by a feeling, and the number says there is time.

### What is guarded

Not a wall clock budget. A test that fails when the machine is busy is a test people learn
to rerun until it passes, and CI is always busy. What is guarded is the shape of the curve:
ten times the input has to cost well under a hundred times the time, which catches
accidental quadratic behaviour and does not care how fast the machine is.

That test earned its place immediately. It found that every unresolved name ran an edit
distance against every name in scope, so a file with ten times as many errors cost ninety
times as much to check. A file full of unresolved names is the normal state of a file being
edited, which is exactly when this principle is about something. Suggestions are budgeted
now, in source order so the output stays the same every time.

**What would falsify this:** a realistic codebase where a full check is slow enough that the
absence of incremental checking stops being a footnote, or a change that makes the scaling
test pass while the constant factor grows enough to miss the target anyway.

---

## P10. Say what is not solved

*Design documents state their open problems and what would falsify them.*

Interop, tooling, and the total absence of an ecosystem are real, and pretending otherwise
just wastes the time of anyone who takes this seriously.

### What a run costs

The other one, and the one people ask about first. There is no code generation: a tree
walking interpreter runs `test` blocks and `main`. Nothing had measured what that costs,
which meant the question everybody starts with, whether a compiler should be a bytecode
machine or native code, had nothing under it. The question in front of it is what a run
spends its time on now.

`cargo run -p deed-driver --example interpreting --release` asks. On one machine:

```text
100000 turns
                      total      per turn   added
-----------------------------------------------------
setup, no turns       0.6ms      5ns
a turn                3.6ms      35ns       29ns
  + an operator       7.8ms      78ns       42ns
  + a field read      10.5ms     104ns      26ns
  + a call taking none 9.9ms      99ns       21ns
  + a call            20.9ms     208ns      130ns
  + another argument  27.5ms     274ns      66ns
  + a contract on it  38.4ms     384ns      175ns

100000 turns, one call in the body, by how many arguments it takes
                        total      per turn   added
-------------------------------------------------------
an operator on a literal 5.9ms      58ns
  + a name instead      7.3ms      72ns       14ns
a call taking nothing   11.6ms     115ns      57ns
  + one argument        19.4ms     194ns      78ns
  + a second            26.4ms     264ns      70ns
  + a third             25.4ms     254ns      0ns
  + a fourth            34.0ms     340ns      86ns

an argument, averaged over the four: 56ns
a name read, from the first two rows: 14ns

50000 pushes, onto a list of this length
length     total      per push   over an empty one
-----------------------------------------------------
0          21.6ms     431ns
16         25.8ms     515ns      83ns
64         31.2ms     624ns      192ns
256        56.7ms     1133ns     702ns
1024       151.7ms    3033ns     2602ns
```

Four things fall out of it and none of them is the one that was expected.

**The walk is cheap and it was not obvious that it would be.** A turn is 32ns, an operator
52ns, a field read 15ns. The first time this was measured they were 88, 94 and 28, and
nothing about the tree walker changed in between. Two things did. A call used to rebuild the
row its declaration promised, look up what each parameter binds, and capture two maps for
`old(...)` and `unchanged(...)` on functions with no `ensures` to read either, all of it once
per call although none of it can change between two calls to the same function. And every
name in the language was hashed with SipHash, which is chosen to survive an attacker
picking the keys, on keys that are a pair of byte offsets out of a file the compiler was
handed. Neither of those is a machine.

**What a call costs is its arguments, and not the names in them.** A call to a function
taking nothing is around 20 to 55ns, which is an operator or less. Each argument after that
is about 56ns, averaged over four of them because a single step at this size is inside the
noise.

The first table used to be read as saying the first argument costs three times what the
second does. It does not. Its two call rows call different functions: `nothing()` returns a
literal and `itself(n)` returns its parameter, so the step from no arguments to one adds a
name read in the callee that the step from one to two does not add. Holding the callee still
at `-> Int { 1 }` and only changing the arity, the arguments cost about the same as each
other, which is what an argument list and a binding per argument would predict.

The same table prices a name read on its own, since `sum + 1` and `sum + n` differ by one and
by nothing else. It is about 14ns, which is roughly a field read. That was the thing this
section previously called the most expensive small thing in the language, on the grounds that
it is two lookups where a field read is one. Two lookups it still is, and it is not expensive:
the hash change took the cost out of it. About a quarter of an argument is the name; the rest
is the argument list and the binding.

**Copying a list is nearly free per element and not free per call.** Values are immutable, so
`push` hands back a copy, and that copy is what everyone points at. It is about 2.5ns an
element. A list has to get past a hundred elements before the copy costs what entering the
built-in cost before a single element was touched, and the lists in `examples/logs.deed` are
words in a line and keys in a table: six and four.

**A real program is calls.** `examples/logs.deed` costs about 24us a line, flat from 240
lines to 1920, so nothing in it is accidentally quadratic. It was 34us before the two changes
above and not a line of it moved. Splitting one line into its words is 6us of that, and one
line's worth of splitting is one declared call, one `split`, nine turns and fifteen more
entries into a built-in.

**So the shape question still has nothing to decide between.** What a machine shape decides
is how an expression is dispatched, and a run got about a third faster without one being
chosen. The two things that did it were a table worked out once instead of every time and a
hash function; both are things a bytecode machine would also have needed, and neither is a
reason to prefer one machine over another. Choosing between two on this evidence would be
choosing on the strength of a straw man. One thing was tried and reverted because nothing
moved: a pool that stopped a call allocating a frame, which is how the allocation was ruled
out rather than argued about. And one thing did less than expected: the hash change is in the
resolution tables a check reads too, and `edit_loop` came out the same either way. Whatever a
check spends its time on, it is not this.

**Giving every name a slot is not the next thing, and there is a number saying so.** This
section used to end by proposing exactly that: make a name read cost what a field read costs,
since a field read is one lookup into a map already in hand and a name read is two into maps
that are not, and then measure again to see whether dispatch comes out on top. Measuring the
name read directly rather than inferring it from the cost of a call answers it without the
work. It already costs about what a field read costs. There is no third of a call sitting
behind it to be recovered, so a slot per name would move the total by whatever part of 14ns
an indexed load saves over a hash lookup, and the argument list and the binding it does not
touch would still be there.

**What would falsify this:** take the other three quarters of an argument, the list and the
binding, and make them cost nothing, then measure again. A frame pool was already tried
against the allocation half of that and nothing moved, so what is left is the binding itself,
which is a hash insert per parameter into a map that is thrown away when the call returns. If
removing it puts dispatch on top then the shape question has a basis. If it does not, code
generation was never the next thing and there will again be a number saying so rather than a
preference.

---

## Known tension

P2 (small spec) and P6 (illegal states unrepresentable) pull in opposite directions. Rich
type systems are how you make illegal states unrepresentable, and rich type systems are
large.

There is no clean resolution. The working rule is that a type system feature earns its
place only if it removes an entire class of runtime failure, and features that merely
express something more elegantly get cut. Expect this to be the argument behind most
rejected proposals.
