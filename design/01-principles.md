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

There is no corpus of Vow code and there will not be one for a long time. A language nobody
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

`vow fmt` is what this means in practice. It has no options for the output, not "none yet",
and there is a test asserting that every `.vow` file in the repository is already in
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

`vow fix` is what this means in practice, the same way `vow fmt` is for P4. It applies every
fix marked machine-applicable, re-checks, and repeats until nothing changes. It never
applies a fix marked maybe-incorrect, and there is no flag to make it, because a flag for
applying guesses is a flag someone turns on once and then forgets about.

Two of its rules are about declining. Fixes whose spans overlap are both dropped, since
applying either would leave the other pointing at text that moved and no order makes both
right. And a fix that leaves more errors than it found is treated as a compiler bug and
nothing is written, which is the version of the check that catches a fix that was wrong
rather than merely unhelpful.

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

`vow check --timings` reports wall time per pass. On one developer machine, an unoptimised
build checking the seven files in `examples/` takes about 4ms, and a generated module of 800
functions with contracts on half of them takes about 26ms. Those are numbers from one
machine and a debug build, not a guarantee.

They are also not the target. There is no incremental checking, so what is measured is a
full check of a small program, and P9's claim is about the edit loop. Until something
re-checks only what changed, the target above is a statement of intent and this section is
the honest version of it.

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

---

## Known tension

P2 (small spec) and P6 (illegal states unrepresentable) pull in opposite directions. Rich
type systems are how you make illegal states unrepresentable, and rich type systems are
large.

There is no clean resolution. The working rule is that a type system feature earns its
place only if it removes an entire class of runtime failure, and features that merely
express something more elegantly get cut. Expect this to be the argument behind most
rejected proposals.
