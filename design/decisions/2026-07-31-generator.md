# Decision: generators use push-style Yield for now

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

`for` walks a list that already exists. A generator is a producer that hands
elements to the consumer one at a time, so neither side has to build the whole
sequence first. The issue asking for this (deed-lang/deed#608) was blocked on
the resumption decision: true pull-style generators, where the consumer drives
and the producer suspends between yields, require resumable effects
(delimited continuations).

The language does not have resumable effects yet. The question is whether
the existing handler model can express useful lazy production without them.

## Decision

Implement a push-style `Yield` effect and demonstrate it in
`examples/generator.deed`. The producer calls `Yield.item(element)` for each
element. The handler's return value is a `Bool` signal: `true` means continue,
`false` means stop. The producer reads that signal through the `for` accumulator
via a `while` condition, so the handler can cut the walk short without the
producer knowing anything about the consumer.

This satisfies the core requirement from the issue: the producer does not build
the output list first. Each element is computed only when the handler is ready
for it, and elements beyond the consumer's limit are never computed.

Checklist outcomes:

- `Yield` effect carrying an element: `fn item(element: Int) -> Bool`.
- The inversion, written in Deed: the push direction is in
  `examples/generator.deed`. The pull direction (consumer asks, producer
  resumes) is not writable without resumable effects and is deferred.
- A program that consumes half a sequence and stops: the `TakeFirst` handler
  returns `false` once it has collected its limit, and the producer's loop
  stops on the next `while` check.
- What happens to the half-consumed producer: the `for` loop exits normally
  when `going` becomes `false`. The producer function returns `false` to signal
  it was stopped early. No cleanup is needed and no resource is leaked.
- Whether `for` can walk a generator: not yet. `for` walks `List` only. A
  pull-style generator would need either an iterator protocol or resumable
  effects. The push-style handler model is the current answer to the same
  demand.

Effects cannot be generic in Deed, so `Yield` carries `Int` in the example.
A program that needs a different element type writes its own effect with the
same pattern.

## Drawbacks (required)

Push-style is not control inversion in the full sense. The producer drives
the walk; the consumer can stop it but cannot ask for specific elements or
restart it. Resuming from where a producer left off is not writable.

The `Bool` return from `Yield.item` is a convention the producer has to honour.
A producer that ignores the signal cannot be stopped. This is different from
true pull-style, where a consumer that stops simply never calls the producer
again.

Effects cannot be generic, so there is no general `Yield<T>` library. Each
element type needs its own effect declaration.

## Rejected Ideas (required)

- Option: implement pull-style generators with resumable effects now.
  - Rejected because: the resumption decision is not yet taken. Adding
    resumable effects changes the interpreter model, the type system, and
    the effect row semantics. That is a separate, larger decision.

- Option: ship `std/generator.deed` as a standard library module.
  - Rejected because: effects cannot be generic, so a shipped module would
    only cover one element type. The pattern is simple enough that each
    program can declare its own effect for its element type.

- Option: extend `for` to walk a generator now.
  - Rejected because: `for` walks `List`. Making it walk a generator requires
    either a structural iterator protocol (not in the language) or pull-style
    generators (needs resumable effects). Neither is ready.

## Open Questions (required)

- Whether and when to add resumable effects, which is the prerequisite for
  pull-style generators and the full OCaml-style control inversion.
- What the iterator protocol should look like if `for` is to walk something
  other than `List`.
- Whether generic effects should be added, which would allow a general
  `Yield<T>` library rather than per-type effect declarations.

## References

- `examples/generator.deed`
- deed-lang/deed#608
- deed-lang/deed#575 (resumption decision, blocked)
- `design/03-effects.md`
