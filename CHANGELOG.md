# Changelog

This file is written per change, not assembled at release time. Its sections
follow the compatibility surface in [design/07-versioning.md](design/07-versioning.md),
which says what a Deed version promises.

## Unreleased

Update this section in the same PR as the user-visible change. When cutting a
release, rename it to the version and date, and publish that section as the
release notes.

### Programs that used to compile and no longer do

- None yet.

### Language

- None yet.

### Diagnostics

- None yet.

### Standard library

- None yet.

### Tools

- None yet.

### Measurements

- None yet.

## 0.2.2 (2026-07-31)

Nothing about the language moved. What changed is what `deed explain` prints
and what the WebAssembly artifact will answer, both of them for the benefit of
something outside this repository reading them.

### Programs that used to compile and no longer do

- None.

### Language

- None.

### Diagnostics

- `deed explain` prints Deed. Thirty of the ninety-seven pages showed a Rust
  `format!` template instead of a program, doubled braces and placeholders and
  all, because the example is copied out of a test and a test is Rust. Every
  page that had an example still has one.

### Standard library

- None.

### Tools

- The wasm artifact answers two more questions. `deed_tokens` says how the
  compiler's own lexer classified each byte range, so a page can colour Deed
  without a second grammar to keep in step, and `deed_explain` hands over
  every diagnostic code with its page, so a site can host an error index
  rather than write one.

### Measurements

- The wasm artifact is 858,097 bytes uncompressed and 280,748 gzipped, up
  from 816,258 and 265,841, inside the same ceiling.

## 0.2.1 (2026-07-31)

One fix, and it is the whole release: 0.2.0's WebAssembly artifact did not
work. Nothing else changed, so there is nothing here to be careful about.

### Programs that used to compile and no longer do

- None.

### Language

- None.

### Diagnostics

- None.

### Standard library

- None.

### Tools

- The wasm artifact answers instead of trapping. Every verb went through
  `check_all`, which read a clock, and `wasm32-unknown-unknown` has none, so
  0.2.0's artifact aborted on any input at all. CI now runs the artifact
  rather than only building and weighing it.

### Measurements

- None.

## 0.2.0 (2026-07-31)

Written after the fact rather than as the changes landed, which is the thing
this file asks for and did not get: 231 commits reached `main` between 0.1.0
and here, and one of them touched this file. That one added the policy.
Reconstructed from the merged work rather than from memory, and the counts in
it are measured.

### Programs that used to compile and no longer do

- A handler must implement every operation of the effect it names, or none of
  them. Half a handler used to check cleanly and then fail at run time.
- A contract may only reach an effect its signature already mentions. Reaching
  further used to check cleanly and then die at run time, inside the callee
  that had declared everything correctly.
- A closure written over a handler's `state` is refused. It used to compile,
  and read a different handler's state than the one it was written under.
- A task cannot outlive the block that started it.
- Two files claiming the same module path is an error, reported on both.
- A name imported into a `uses` clause that is not an effect is an error
  rather than a warning. As a warning it also silenced the rest of that
  function's row checking.

### Language

- Effect handlers are one-shot, and a resumption is a one-shot value. That is
  what decides the dispatch rather than the other way round.
- `finally` on a handler, for the cleanup a block owes when it is left.
- `abandon`, which unwinds a suspended computation and runs the `finally`
  clauses on the way out.
- The generator pattern, which turns out to be a `Yield` effect and no new
  syntax at all.
- Alternative patterns in a match arm. They bind nothing, which is what keeps
  them free.
- An alias may name a shape with a hole in it, and it expands on its way out
  of a module, however many modules it came from.
- `String` length is Unicode scalar values, stated rather than implied.
- Identifiers are UTF-8, with the grammar written down.

### Diagnostics

- 73 codes to 85. New: `AMBIGUOUS_MODULE`, `NO_DETACHED_SPAWN`, `ABANDONED`,
  `CLOSURE_OVER_STATE`, `HANDLER_MISSING_OPERATION`,
  `CONTRACT_EFFECT_NOT_DECLARED`, `BINDING_IN_AN_ALTERNATIVE`,
  `REFINEMENT_TYPE_PARAM`, `DEPRECATED_DECLARATION`, `UNKNOWN_EDITION`,
  `UNKNOWN_DIRECTIVE`, `MISSING_COMPONENT_PATH`. None removed or renumbered.
- Every message the lexer, parser, resolver, effect checker, type checker and
  interpreter can produce was read back in a test. A code having one test does
  not mean its sentences have been read: several codes carried messages that
  nothing had ever rendered, and two of those carried repairs that made the
  program worse.
- A secondary label can name a file of its own, so a diagnostic about two
  modules stops drawing its caret into the wrong one.

### Standard library

- `std/list` and `std/table` ship with the compiler rather than sitting in
  `examples/`.
- `std/map`, a red-black tree, for the case an ordered table was answering
  badly.
- `std/list` gains `find`, `take`, `drop`, `concat`, `flatten`, `partition`,
  `zip`, `unzip`, `enumerate`, `windows`, `chunks`, `intersperse`, `scan`,
  `transpose`, `group_by`, `sort`, `flat_map`, `filter_at` and `fold_at`.
- `std/string` gains `contains`, `replace`, `trim_start`, `trim_end`,
  `to_upper` and `to_lower`.
- `std/table` gains `remove`.

### Tools

- `deed build` writes a WebAssembly module beside the file it was given, and
  `deed build --component` writes a component whose WIT world is derived from
  the program's effect rows rather than written by hand beside them.
- `deed run --compiled`, which maps a trap back to the span that caused it.
- `deed run --profile-runtime`, attributing cost to functions, contracts and
  handlers separately.
- `--lock` and `--locked`, for builds that are offline and say what they used.
- A wasm entry point a page can call: `check`, `run`, `test` and `fmt` over
  linear memory, with the CLI's own JSON and no binding generator.
- The language server gains document highlight, folding ranges, inlay hints,
  go to implementation, type definition, document links and selection ranges,
  and the VS Code extension now starts it.
- A tree-sitter grammar, which is what Helix and Neovim colour with.
- An external conformance suite and a harness that runs it.
- Fuzzing: a corpus replayed on every build, and a scheduled run that adds to
  it.

### Measurements

- The wasm artifact is 816,258 bytes, 265,841 gzipped, against a ceiling of
  1,500,000 and 550,000.
- A handler operation costs roughly 128ns over a plain call when the handler
  holds no state, 142ns when it reads some. Not enough to justify a
  tail-resuming fast path before a compiled backend can measure its own.
- `std/map` against `std/table`, benchmarked, with the decision recorded.
- Compiled memory growth, and the slopes of the edit loop and the runtime, are
  both ratcheted in CI rather than hoped for.

## 0.1.0 (2026-07-27)

Initial public release.

### Programs that used to compile and no longer do

- Not applicable in the initial public release.

### Language

- Initial public release of the language and compiler.

### Diagnostics

- Initial public release of the compiler's user-facing diagnostics.

### Standard library

- Initial public release of the shipped `std/` modules.

### Tools

- Initial public release of the `deed` CLI and editor tooling.

### Measurements

- The generated `v0.1.0` release notes included measurement work; future
  releases record the measurements that changed a decision here.
