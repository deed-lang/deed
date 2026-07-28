# Contributing

Thanks for looking. This is very early, so what is useful right now is probably not what
you expect.

## What helps most

**Attack the design.** The documents in [`design/`](design/) are the actual product at this
stage. If something does not hold up, saying so is worth more than any amount of code.

The claims most likely to be wrong, in order:

1. **Context radius of one.** The whole language is built on the idea that a signature can
   carry everything you need to verify a body. If you can write a realistic program where
   that falls apart, that is the most valuable thing you could contribute.
2. **Effect ergonomics.** Every effect system before this one died on annotation burden. If
   the design in [03-effects.md](design/03-effects.md) drowns real code in rows, it dies the
   same way.
3. **Capability plumbing.** Threading capabilities through deep call stacks has been tried
   and people hated it. [04-capabilities.md](design/04-capabilities.md) has no answer yet.
4. **Contracts being as easy to get wrong as implementations.** If that is true, reviewing
   the contract instead of the body buys nothing and the language is just slower.

Open an issue. A concrete counterexample beats an opinion, but an opinion beats silence.

## What does not help yet

- Feature requests. The specification has a size budget (P2 in
  [01-principles.md](design/01-principles.md)) and it is the main thing standing between
  this and every other language that collapsed under its own surface area.
- Syntax bikeshedding. Syntax deliberately copies Rust and TypeScript wherever possible,
  because recognition is free and novelty is not.
- Compiler PRs against things that have no agreed design. There is nothing to build against
  yet for most of it.

## If you want to write code

Check the [roadmap](https://github.com/deed-lang/deed/issues/1) and the open issues. Work is
tracked there, and anything actionable is labelled.

The compiler is Rust. Before opening a PR:

```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Tests

**No assertion may be satisfied by an empty collection.** If a test asserts something about
every element of a list, or about no element of one, then the empty list satisfies it and
the test says nothing. So whenever a test builds a collection and then asserts over it, it
also has to establish that the collection is not empty, from inside the test.

This is about any collection, not about directories. It has been fixed three times here and
written down three times as a rule about walking a directory, and both times it came back it
was somewhere else: an `all(...)` over the obligations a check produced, and a slice from
index one over the lines a program printed, which is empty whenever the program prints one
line. Naming the container is what let it come back, so the rule is about the shape.

The shapes to look for, all of which hold for free on nothing:

- `xs.iter().all(...)`, and `!xs.iter().any(...)`
- `xs.is_empty()`, where what is being claimed is that nothing in `xs` is wrong rather than
  that `xs` is empty
- `xs[n..]`, and anything else that can slice down to nothing
- a loop with the assertions inside it

An `any(...)` asserted true is fine, because that one fails on nothing. So is asserting a
collection is empty when emptiness is the claim itself, as in "this raises no obligation".

Two ways to establish the collection is real. Assert a count first, which is best when the
count is knowable and worth pinning down, and otherwise count what the loop actually
compared and assert that came to more than zero. Prefer routing through a helper that
already makes the guarantee over writing a fresh assertion beside it; if a test cannot use
the helper, say why in the test.

Verify a strengthened assertion by breaking the thing it now guards and watching it fail by
name. An assertion nobody has seen fail is an assertion nobody has read.

## Commits and PRs

Conventional commits, so history stays greppable:

```
feat(lexer): tokenize contract keywords
fix(parser): handle trailing comma in ensures
docs(design): clarify effect propagation rules
```

One PR, one concern. Link the issue it closes. If a PR changes behaviour that a design
document describes, update the document in the same PR, because a design that lags the code
is worse than no design at all.

## Design changes

Anything touching `design/` goes through a PR with the reasoning in the description. Include
what the change rules out, not only what it enables. A principle that never rejects anything
is decoration.

## Code of conduct

Criticism of ideas is the point of this repository; criticism of people is not. That is the
whole policy, and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) is what it means in practice.

## Security

The `Dir` sandbox and capability safety are claims this project makes out loud, so a way
around either is a security report rather than a bug. [SECURITY.md](SECURITY.md) says where
those go and what is in scope.

## AI assistance

This project is developed with AI assistance and says so openly. If you use it for a
contribution, no need to make a thing of it, but do not open a PR you have not read and
understood yourself. The entire premise here is that review is the expensive part, and it
would be a poor look to skip it.
