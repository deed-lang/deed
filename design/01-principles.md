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
