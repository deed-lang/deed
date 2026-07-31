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
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --doc --workspace
```

[cargo-nextest](https://nexte.st/) rather than `cargo test` because this workspace has forty
test binaries and nextest runs them at once. It cannot run doctests, which is why they are
the second line rather than folded into the first. Nextest also runs with `--no-fail-fast`
by default (`cargo test` does not): without it, the first failing test binary stops every
one after it, and a pull request can look like it broke one thing when it broke several.

This machine's own build constraint: there is no MSVC linker here, so every `cargo` command
needs `[build] target = "x86_64-pc-windows-gnullvm"` set in `~/.cargo/config.toml` (llvm-mingw
provides the linker). That is local to whoever is missing MSVC, not a repository setting;
a machine with MSVC or on Linux/macOS needs none of it. `cargo check --target
wasm32-unknown-unknown` additionally needs `rustup target add wasm32-unknown-unknown` once,
for anything touching `deed-wasm`.

## The ratchets

This repository's tests are stricter than most, in ways nothing here will explain until CI
rejects a pull request that looked reasonable. All of it is deliberate: a design document, a
README transcript or a doc comment that nothing checks goes stale in hours, and every one of
these exists because that happened here at least once. None of it is arbitrary, and knowing
the list up front is cheaper than meeting it one CI run at a time.

- **Adding a `.deed` file to `examples/`** can fail three separate ratchets at once: the
  README's example list, the README's passing-test count, and the spelled-out file count in
  [01-principles.md](design/01-principles.md) all read the corpus and compare. Update all
  three in the same PR, or run the tests locally and fix whichever one names itself.
- **Adding a diagnostic code without a test fails the build**, and names the code:
  `every_diagnostic_code_is_named_by_a_test` (`crates/deed-driver/tests/codes.rs`) reads
  every crate's `codes.rs` and every crate's `tests/`, and a testless code is a message
  nobody has confirmed is reachable, correctly worded, or pointed at the right span.
- **Adding a function to a shipped `std/` module fails unless one of that module's own
  tests names it**, in `crates/deed-driver/tests/shipped.rs`: a module can carry more
  functions than its tests ever call, and "wrote a test" is not the same claim as "wrote a
  test that exercises this one".
- **A number written in prose in a design document is probably checked** against something
  the compiler can count: `crates/deed-driver/tests/documentation.rs` reads sentences like
  "the prelude is twenty-two names" or "nine of the sixteen crates" back out of `design/*.md`
  and `README.md` and compares them to a real count. A number nothing checks goes stale
  first and is noticed last.
- **Changing a keyword fails the grammar test in both directions.**
  `crates/deed-parser/tests/grammar.rs` compares the compiler's own keyword and soft-keyword
  lists against `editors/vscode`'s TextMate grammar, so a keyword can be missing a colour or
  the grammar can invent one that colours something that is not actually a keyword.
- **Formatting is not configurable and is checked, not suggested**: `cargo fmt --all --
  --check` is part of CI, and `crates/deed-fmt/tests/repository.rs` additionally requires
  every `.deed` file in this repository to already be in the formatter's own canonical form.

What to run before pushing, in the order CI runs them:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --doc --workspace
```

A pull request also runs `cargo mutants --in-diff` against only the lines it touched (see
"Tests" below); nothing to run locally for that beyond the tests above, since a mutant
surviving means one of those tests should have caught it and did not.

- **`CHANGELOG.md` is part of the release contract**: `crates/deed-driver/tests/documentation.rs`
  checks that the changelog still carries the named user-facing sections and that the
  release workflow reads release notes from it rather than generating them from commit
  titles. If a PR changes what a user sees, update `## Unreleased` in the same PR.



## Tests

**No assertion may be satisfied by an empty collection.** If a test asserts something about
every element of a list, or about no element of one, then the empty list satisfies it and
the test says nothing. So whenever a test builds a collection and then asserts over it, it
also has to establish that the collection is not empty, from inside the test.

This is about any collection, not about directories. It has been repaired three times here
(`2944fab`, `4e9151f`, `44082b4`) and each time the reasoning was written as a comment
beside the one assertion being repaired, which is the right place for it and the only place
it went. Two of those three are not about directories at all: one pins the number of
diagnostic codes a test parsed out of the source before comparing anything, and one asserts
that a perturbation actually perturbed something. So the general shape was understood, it
was just never stated anywhere a test author would meet it before writing a test, and the
one commit message that did generalise put the boundary in the wrong place: "only walking a
directory can quietly produce nothing". It came back twice somewhere else, as an `all(...)`
over the obligations a check produced and as a slice from index one over the lines a program
printed, which is empty whenever the program prints one line. Naming the container is what
let it come back, so the rule here is about the shape.

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

That was done by hand here, one edit at a time, which meant a handful of breakages got tried
per change and nobody asked about the rest. [cargo-mutants](https://mutants.rs/) asks about
all of them:

```
cargo mutants --file crates/deed-typeck/src/check.rs
```

It edits one thing, builds, runs the tests, and reports every edit that no test noticed. The
first file it was pointed at here produced fifty-one edits and fourteen that nothing caught,
in under two minutes.

CI runs it on the lines a pull request touched, and fails when one of them can be broken
without a test noticing. Only the diff, deliberately: the point is not to hold the whole
tree to a standard it has never met, it is that new code arrives with tests that would
notice it being wrong. A mutant reported as `unviable` did not compile and is not a finding.

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

## Changelog

`CHANGELOG.md` is written per change, not assembled at release time.

If a PR changes anything user-visible that release notes should carry, update
`## Unreleased` in the same PR, under every heading it touches:

- `Programs that used to compile and no longer do`
- `Language`
- `Diagnostics`
- `Standard library`
- `Tools`
- `Measurements`

Anything that makes an old program stop compiling goes in `Programs that used
to compile and no longer do`, even when the refusal came from the parser, type
checker, `std/`, or a tool. That is the heading a person scans first when
checking whether an upgrade broke a program they already had.

Those headings match the compatibility surface in
[design/07-versioning.md](design/07-versioning.md). Patch releases should leave
the refusal section empty. At release time, rename `Unreleased` to the version
number and date; the release workflow publishes that section as the GitHub
release notes.

## Fuzzing

The fuzz target is `check`: it runs arbitrary text through the full check
pipeline and fails on any panic. The corpus lives in
`crates/deed-driver/fuzz/corpus/check/` and is replayed on every build by
`crates/deed-driver/tests/fuzz_corpus.rs` (no fuzzer needed, stable toolchain).
The corpus grows automatically: the scheduled workflow in `.github/workflows/fuzz.yml`
runs a bounded 30-minute fuzz session every Sunday and commits any new inputs
it discovers.

To run the corpus replay without the fuzzer:

```
cargo test -p deed-driver --test fuzz_corpus
```

To run the fuzzer locally (nightly toolchain and `cargo-fuzz` required):

```
cargo install cargo-fuzz
cd crates/deed-driver
cargo +nightly fuzz run check -- -max_total_time=60
```

If the fuzzer finds a crash, add the reproducer to the corpus so it is
replayed on every future build:

```
cp fuzz/artifacts/check/crash-<hash> fuzz/corpus/check/
```

## Design changes

Anything touching `design/` goes through a PR with the reasoning in the description. Include
what the change rules out, not only what it enables. A principle that never rejects anything
is decoration.

Large proposals and decisions live in [design/decisions/](design/decisions/). Start from
[design/decisions/TEMPLATE.md](design/decisions/TEMPLATE.md). Every record requires
`Drawbacks`, `Rejected Ideas` and `Open Questions`.

If a decision changes later, update the original record in the same PR: set status to
`Superseded` and add a `Superseded by` link to the replacement record.

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
