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

- `a and b` and `a or b` are read as `&&` and `||`, with DEED2020 naming the
  operator and `deed fix` writing it. The words are ordinary names here, so a
  `where` clause holding one used to stop at the word and answer with the block
  that did not follow it. The resolver has had the answer since #213 and never
  got to give it: across eighteen recorded model runs the word reached the
  parser seventeen times and the resolver none.
- A function body written without braces gets DEED2021, which says a body is a
  block and offers the braces, and the body is read to the next declaration so
  the function is still a function. It used to be a message about a brace, then
  a message about the first line of the body not being a declaration, then a
  return type that matched nothing. Thirty-two of the six hundred programs in
  the recorded model runs were written this way.

### Standard library

- None yet.

### Tools

- `deed check` no longer panics on a function whose parameter list never
  closes. A declaration used to end at the token sitting where its closing
  token should have been, so a signature could reach past the start of its own
  body, and `deed fix` builds the region a `uses` clause goes in by subtracting
  one from the other. Every construct now ends where the parser actually read
  to. Found by the nightly fuzzer.
- The scheduled fuzz run reports what it finds as an issue rather than as a
  pull request. Opening a pull request from a workflow needs a repository
  setting that also lets workflows approve pull requests, and the first run
  that found anything got told so and said nothing to anybody. The branch is
  pushed either way, so the issue names it.

### Measurements

- None yet.

## 0.2.3 (2026-08-01)

Most of this release was found by pointing three models at the compiler
through `deed mcp` and reading what they were told. A diagnostic that names a
mistake and then throws the rest of the file away costs the reader more than
the mistake did, and nine of these turned out to be that. The one that is not
about wording is the checker: it could not use a `where` clause about a sum,
so it answered Guarded to a function whose precondition was word for word its
own obligation, and the transcript shows a model taking that answer and
writing a worse contract because of it.

### Programs that used to compile and no longer do

- Naming a generic prelude function instead of calling it is refused. Five
  names work on any type, so there is no one signature to hand back, and they
  were typed `Unknown` instead. `Unknown` absorbs, so `at == n` compared clean
  against an `Int` and reached an interpreter that had no value for it either.
  DEED4019 now, with a note saying to call it.
- `with H { ... }` is checked where the handler is installed. The form with
  braces was already checked as a literal; the form without one parsed as the
  handler followed by a block, so missing state waited for the interpreter and
  arrived as DEED6006 attached to whichever test ran first. Whether a value
  was written is a fact about the source, so it is answered there.

### Language

- A `where` clause about two names added together is a fact the checker keeps.
  It held a range per name and a range for the difference of a pair, because
  `low < high` is about neither name on its own, and a sum had nowhere to go.
  So `where count + delivered > 0` was read and dropped, and returning
  `count + delivered` into a `value > 0` refinement came back Guarded while
  the strictly stronger `count > 0, delivered > 0` proved. Two names and only
  two: three is a shape this does not hold, and Guarded stays the answer.
- A handler frame is given back when its block ends. Frames were never freed,
  so a `with` inside a walk allocated once per turn and a program installing a
  handler in an ordinary loop ran out of linear memory. A frame's lifetime is
  exactly its `with` block, which is what `with` means rather than something
  inferred, so they get their own region and the block rewinds it. Values do
  not follow: a block's value outlives the block.

### Diagnostics

- A list written one to a line with no commas says so, once per comma, and
  goes on reading the list. Match arms used to cost nine diagnostics and none
  of them said comma; a `choice` said "insert `}`", which is an answer to a
  question nobody asked and a repair that would have made it worse. DEED2015,
  with three sentences, one each for arms, variants and fields.
- `->` where `=>` belongs is named rather than expected. DEED2016, with the
  right arrow offered and the arm read as an arm afterwards.
- An `ensures` clause that names no outcome says so once. The condition
  standing where `ok =>` belongs used to cost two diagnostics, the second one
  about the `=>` that never came.
- A constraint written on a parameter says which clause it belongs in, and the
  expression is read into that clause rather than thrown away, so the names in
  it resolve. DEED2017. A `type` carries its refinement inline, so writing the
  same thing on a parameter is a fair guess.
- `xs ++ ys` and `x :: xs` are named, with the call from `std/list` that does
  it and which argument the list is. DEED2018.
- A value on a handler's state says where the value goes instead of ending the
  handler. DEED2019, and the operations after it are still operations.
- `cannot find X in this scope` names the shipped module that declares the
  name and writes the `use`. `fold` is in `std/list`, compiled into the binary
  that just said it could not find it. A name two shipped modules declare gets
  the sentence and no repair.
- Assigning to a name inside a `for` says what to write. DEED4015 said handler
  state is the only mutable thing here, which is right and does not say what
  the shape is, so it now spells the accumulator out with the reader's own
  names.
- A handler's `state` declaration says what one looks like, both halves of it,
  rather than naming the token it wanted.
- An unhandled effect that reached `main` says it is the program's boundary.
  `deed run` used to advise a `with` block, which would discharge the effect
  and take the import back out of the component's world.
- A guarded obligation says why it is guarded, including when nothing tried to
  prove it, which used to be said by saying nothing at all.

### Standard library

- `std/date`, a calendar. `date_of` refuses a clock set before 1970 rather
  than answering, `is_leap_year` and `days_in_month` for a caller walking a
  year, and `text` as `YYYY-MM-DD`, ordered so that sorting the text sorts the
  dates.
- `std/ratio`, exact fractions, as a library rather than a number type.
- `std/list` gains `sum` and `largest`. `fold`'s own comment said a library
  with `map` and `filter` and no `fold` stops working the moment somebody
  wants a sum, and then the file did not have the sum. `largest` takes the
  comparator `sort` takes, so one function covers both ends.

### Tools

- `deed doc <path>...` writes the API page any shipped module gets, for any
  module, as Markdown on standard output. The generator lived in a test file.
- `deed mcp`, a server that hands an agent the compiler's six verbs. Its
  handshake names every tool it offers and says why checking is not the last
  step.
- The wasm artifact runs the tests a contract generates, not only the ones a
  person wrote, and reports each property with its seed. It also refuses a
  module that does not check, and ends a test run with a summary line, because
  silence already meant "well formed" on that surface and could not also mean
  "nothing ran".
- The wasm artifact says why there is nothing to run. It answered "no `main`
  found" where the CLI says "no `main` found, so there is nothing to run", and
  most of the corpus is libraries, so that is the answer a reader is most
  likely to meet first. One constant now, used by both.

### Measurements

- The edit loop was re-measured. 82us a file and 42ms at 512 files, against
  the 59us and 30.1ms the design document stated, so the trigger for writing
  an incremental cache is about 1,200 files rather than about 1,700. The
  conclusion does not change and a third of the claimed headroom was not
  there. The arithmetic on top of the table is now a test.
- Something other than the author writes Deed, and it is measured.
  `benchmarks/` runs six tasks through a model twice, once with the compiler
  and once without. Without it, three models answered 0 of 6 across every run.
- The conformance suite has twenty-eight cases in three groups rather than
  four, which was an example of a format rather than a suite.

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
