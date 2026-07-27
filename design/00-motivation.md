# Motivation

## The premise

The cost of producing code has collapsed. The cost of trusting it has not moved at all.

That sounds like an observation about tooling, but it is actually a statement about which
language features are worth paying for, and the answer has changed.

## The cost model

When a person types code, the dominant cost is keystrokes and working memory. Languages
optimized for that: terse syntax, inference everywhere, implicit behaviour so you write
less, clever shorthand so common things are short.

When code is generated, the cost looks like this instead:

$$
\text{cost} = \underbrace{T \times N}_{\text{generation}} + \underbrace{C_{\text{review}}}_{\text{human}} + \underbrace{P_{\text{escape}} \times C_{\text{prod}}}_{\text{failure}}
$$

where $T$ is tokens per attempt, $N$ is attempts until correct, $C_{\text{review}}$ is what
it costs a person to convince themselves the result is right, $P_{\text{escape}}$ is the
probability a defect reaches production, and $C_{\text{prod}}$ is what that costs.

$T$ is the term everyone reaches for, and it is the smallest one. $N$ is a multiplier.
$P_{\text{escape}} \times C_{\text{prod}}$ dwarfs both.

So optimizing for brevity is optimizing the wrong variable. The right objective is:

> converge in the fewest attempts, and do not let a wrong program escape.

## Redundancy is a feature

This is the part that feels backwards.

Treat the implementation as a message and the thing producing it as a noisy channel.
Types, preconditions and effect declarations are redundant with the body: they restate, in
a different form, something the body already implies. In coding theory that redundancy is
exactly what lets you detect corruption.

A language with no redundancy has no error detection. One wrong token produces a wrong
program and nothing notices. A language with checked redundancy catches the mistake at
compile time, which is the cheapest place a mistake can happen.

The important word is *checked*. Comments are redundancy too, and they are worthless,
because nothing verifies them and they drift into lies. Types are worth something for
exactly one reason: they are enforced.

The practical consequence, with made up but directionally honest numbers:

| | terse and dynamic | explicit and checked |
| --- | --- | --- |
| tokens per attempt | 400 | 800 |
| attempts to correct | 3.5 | 1.3 |
| total tokens | 1400 | **1040** |
| where defects surface | production | compile time |

The verbose option wins on both axes. Verbosity was never the problem. Ambiguity was.

## What we are actually optimizing

Four measurable things. Every design decision in this repository should be traceable to
one of them.

### M1. Context radius

How many other definitions must be read to write or verify this one. **Target: one, the
signature itself.**

Everything that inflates this is a defect in the language, not a feature: global mutable
state, implicit conversions, inheritance chains, dependency injection containers,
exceptions that cross frames, decorators that rewrite what they wrap, "this field gets set
by a middleware somewhere". All of them mean you cannot understand a function by looking
at it.

### M2. First-attempt correctness

A direct function of ambiguity. If there are five ways to express something, output is
inconsistent and review has to start over each time. One canonical form is an engineering
requirement here, not a style opinion.

### M3. Error to fix distance

Does the diagnostic point at the edit that fixes it, or does it say `type mismatch` and
send you through nine files? Compiler output is not a courtesy to a human reader any more.
It is the API of a feedback loop, and it should be structured, one failure at a time, and
carry an applicable patch where one exists.

### M4. Blast radius

Where does a wrong guess surface. Compile time is free, test time is cheap, production is
not. The job of the type system is to drag that arrow leftwards.

Multiplying all four: **build and test latency**, because it is paid once per attempt and
attempts are the multiplier. Go's 2009 argument that compilation speed is a language
feature is more true now, not less.

## Why not just a library

Every piece of this exists as a library somewhere. Contracts, effect tracking, sandboxing,
durable execution. They are all bolt-ons, and they all share the same failure: they are
optional.

- A contract library checks the functions that opted in. The bug is in the one that did not.
- An effect library describes effects it knows about. Ambient authority means any function
  can do anything regardless.
- A sandbox at the process boundary tells you nothing about a module. It is also slow
  enough that nobody runs it per test.

Absence has to mean something. "This function performs no IO" is only useful if the
compiler guarantees it, and a compiler can only guarantee it if the rule is total. Total
means it is in the language.

## Why now

Two things changed at once.

Formal methods have been technically viable and practically dead for thirty years, and the
reason was always annotation cost. Nobody would write the invariants. That cost is no
longer what it was, and the main barrier to verification was economic, not technical.

At the same time, generated code is being executed automatically, which turns "what is this
function allowed to touch" from an academic question into an operational one.

Neither of those is a small shift, and no existing language was designed with either in
mind.

## What would falsify this

Worth stating up front, because a design that cannot be wrong is not a design.

- If contracts turn out to be as easy to get wrong as implementations, the review argument
  collapses and Deed is just a slower language.
- If effect and capability annotations propagate so painfully that real programs drown in
  them, the ergonomics kill it, the way they have killed every effect system that stayed in
  academia.
- If it turns out generated code is accurate enough that catching defects early stops
  mattering, then the whole cost model above is solved by someone else's progress.

The first two are the ones to watch, and both are reasons to build something runnable
before writing much more prose.
